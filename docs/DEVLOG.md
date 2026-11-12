# Dev log

What changed, why, and the numbers, per wave or merged change. Newest first. Add an entry for
every wave or merged change ([AGENTS.md §9](../AGENTS.md#9-dev-log)); details live in the commits,
the [CHANGELOG](../CHANGELOG.md) and the docs. New entries name their commit by short hash
(`git show <hash>`) once it is known; the entries below predate the current history and carry none.

## 2026-11-11

### M8.1 headless: recovery journals, doc.recovery.*

- **What:** `pbi_ops::recovery` (`RecoveryStore`: versioned JSON-lines journals in the per-user
  data folder, created `0600`, a cut-short last line tolerated, foreign or newer formats refused).
  `Registry::execute` journals each mutating command that changed the revision, under the
  document's lock, and deletes the journal on `doc.save`, `doc.saveAs`, `doc.reload` and
  `doc.close`; journal errors only log. A journal starts only from a saved document and tracks its
  undo/redo depth: an undo past its start, or edits made while recovery was off, stop journaling
  until the next save, so every journal replays from disk. `doc.recovery.list`, `doc.recovery.restore` (reopen with
  the header's `doc.open` parameters and replay through the registry; replay relies on the
  per-document deterministic object ids) and `doc.recovery.discard`. Off unless the host calls
  `Workspace::set_recovery` (ADR-0013 §5). The journal holds commands, not the undo stack; the
  replay makes the restored edits undoable again.
- **Why:** M8.1, after M8.2 and M8.3 (ROADMAP "Next").
- **Numbers:** 3 unit tests, `crates/pbi-ops/tests/recovery.rs` (4 tests: seven edits survive a
  dropped workspace and replay to the same measures, a failed replay keeps the journal, untracked
  edits and undos past the start drop it, off by default); catalog +3 commands.
- **Open:** the app's opt-in, startup prompt and Options page.

### M8.3 headless: semantic diff, doc.diff

- **What:** `pbi-core::diff` (`diff_documents`): matches model and report objects by name
  (case-insensitive; relationships by name, pages and visuals by their PBIR name) and compares
  their serialized properties without ids, origins or child collections, so formatting never
  counts. `doc.diff` (pbi-ops) runs it between the current revision and the files on disk
  (`Session::read_path`), another open document or a path.
- **Why:** M8.3, the next step of M8 (ROADMAP "Next"); the Compare view and Keep mine/Compare of
  M8.2 build on it.
- **Numbers:** 4 unit tests, `crates/pbi-core/tests/diff.rs` (an Acme copy with six scripted
  changes and one reformatted visual: exactly six objects), 1 test in `crates/pbi-ops/tests/save.rs`;
  catalog +1 command.
- **Open:** TMDL/PBIR text snippets per change and the app's Compare view.

### M8.2 headless: doc.status, doc.externalChanges, doc.reload

- **What:** three `pbi-ops` workspace commands over new `pbi-session` methods.
  `Session::external_changes` rescans the PBIP fingerprint against the open/saved baseline;
  `Session::reload` rereads the document in place (same resolver, clock, reader options) and
  publishes it only after the read succeeds. `History::reset` (pbi-model) starts a clean history at
  the next revision, so a reload never rewinds it (ADR-0012). `DocEntry::reload` also forgets the
  loaded data and the canvas state; `doc/changed` gains `reloaded`. `doc.reload` is not
  agent-safe (ADR-0012 §6) and refuses unsaved edits without `discardChanges` (ADR-0013 §3).
- **Why:** M8.2, the first step of M8 (ROADMAP "Next"); `doc.status`/`doc.reload` moved there from M6.
- **Numbers:** 2 new tests in `crates/pbi-ops/tests/save.rs` and 1 in pbi-model; catalog +3
  commands.
- **Open:** the file watcher with notifications, the app's notification bar, the Compare view
  (M8.3) and recovery (M8.1).

### M evaluator: free values that contain their own slot

- **What:** `pbi-m` registers every thunk (as a `Weak`, pruned like frames) and
  `Interp::release` releases them: a released thunk drops its memoized value and reads as
  "released". Values that contain their own lazy slot (`let r = [a = @r] in r`, `{@r}`, a shared
  query that refers to itself in a record or list, or returns `#shared`) formed a thunk → value →
  thunk `Arc` cycle that clearing frames never broke. A `cfg(test)` count of live thunks and frames
  backs a new test over these shapes (`leak_tests` in `crates/pbi-m/src/eval/mod.rs`); two
  `regression-*` seeds under `fuzz/seeds/m_eval/`.
- **Why:** the nightly Fuzz job failed on 2026-11-10 and 11-11 with `m_eval` leaks (a shared
  query `#shared`); a red `main` comes first (AGENTS.md §8).
- **Numbers:** the test finds 7 leaks across 9 cases without the fix and 0 with it; `pbi-m` and
  `pbi-session` tests pass. P05 357 ms with the fix against 359 ms without it, and P11 184 ms, on
  the same Windows machine (AMD, 12 cores).
- **Open:** P05 is over its 250 ms target on that machine both with and without the fix. The
  only committed baseline is a macOS M4 Pro at 149 ms, so this is the hardware, not a
  regression; a Windows baseline would make that visible.

### M6 acceptance 6: the PBIT half

- **What:** `save_as_template_keeps_the_expected_values` in `crates/pbi-ops/tests/e2e.rs`: Acme
  with `Orders[Line Amount]` renamed, written as a PBIT (`convert.pbit`, the Stores report),
  opened in a fresh workspace, refreshed from its CSVs through the blob map; every row count and
  measure of `expected.json` and the Stores page's Total Sales card match. DESKTOP-PLAN: M6's
  command list drops `doc.status` and `doc.reload`, which were never built and belong to M8.2
  (external changes, Reload), and M8's list gains them.
- **Why:** a check of every M6 item after A6 found that only the PBIP half of acceptance 6 had a
  test, and that the plan still listed the two commands under M6.
- **Numbers:** the new test passed against the code already on `main`, so it adds coverage
  without a fix. Both criterion 6 tests pass in 1.6 s.
- **Open:** P09 is 53 ms on the CI runner's quick run, against a 50 ms target. It is within
  target on a Windows desktop (acceptance 12).

### A6: screen readers in CI

- **What:** `.github/workflows/screen-reader.yml` builds `dashcraft-app` on hosted runners and
  runs `scripts/app-checks/screen-reader.mjs` through Guidepup with NVDA (Windows), VoiceOver
  (macOS) and Orca (Linux, Xvfb). The script walks the M6 editing journey and
  the M7 formula bar journey (S6 and S12–S22), records what the reader says at each step, and uploads the
  spoken log, a report written after every step, and screenshots. NVDA gates; VoiceOver and Orca
  are informational ([ADR-0030](adr/0030-screen-readers-in-ci.md), which supersedes ADR-0015
  decision 3). The app fixes it found: the File backstage traps focus (`use:modal`), Option+Q
  reaches the formula bar on macOS, the Ctrl+K Ctrl+I chord survives a released Ctrl and is
  announced, and Esc clears a cancelled draft's problems (`DaxEditor.svelte`).
- **Why:** A6 was the last open criterion of M6 and M7; it needed a person on each OS until
  Guidepup could drive the readers on CI runners. Narrator stays by hand before the 1.0 audit
  (M31.8): its evidence in CI is the UI Automation tree the A1–A5 checks already cover.
- **Numbers:** NVDA passes all 27 required steps of 32 (about 4.5 minutes); VoiceOver 24 to 26 of 32
  in about 13 minutes; Orca 20 of 32 (runs 4 and 5 on PR #66). `formula-keys.mjs` 44 checks (one new, the Esc case), `editing-a11y.mjs` 21.
  Commit `633faa8` (PR #66, squashed).
- **Open:** VoiceOver doesn't read the active suggestion (S14, `aria-activedescendant` on a
  text area), and only sometimes reads the suggestion count (S13) or announces a description's
  apply (S25, likely the `readonly` toggle while saving). No VoiceOver command reads the window title
  (S1). Orca under Xvfb doesn't voice polite live regions. Narrator still needs a person
  before M31.8.

## 2026-11-07

### TMDL writer: case variants of `calculationGroup`

- **What:** `pbi-tmdl`'s writer (`Writer::table`) writes every spelling of `calculationGroup`
  left in a table's untyped properties (the reader takes the first unnamed one as the
  calculation group and keeps the others) after the typed group. It matched only the exact
  spelling, so `calculationgroup` was written among the scalar properties, before the group, and
  became the group on reading back. Regression test `fuzz_findings_round_trip` in
  `crates/pbi-tmdl/tests/robustness.rs` (both writers, the fuzz target's invariants) and the seed
  `fuzz/seeds/tmdl/regression-calc-group-keyword-case.tmdl`.
- **Why:** M7 acceptance 4: a manual run of the `Fuzz` workflow on `main` (`f7afc32`, 60 s per
  target) to cover the language-service targets added in M7. `dax_service`, `m_service` and
  `tmdl_service` had no findings; the `tmdl` round-trip target found this one (a 353-byte input,
  minimized by hand to 6 lines).
- **Numbers:** the test fails without the fix and passes with it; `pbi-tmdl`, `pbi-core` and
  `pbi-ops` tests pass unchanged (canonical output changes only for tables with such a property).
- **Open:** A6 for M7.

### M7.6 `dashcraft lsp`

- **What:** a Language Server Protocol 3.17 server, `dashcraft lsp [--stdio] [--model PATH]`
  (`crates/dashcraft-cli/src/cmd/lsp.rs`, `src/lsp/`): `transport.rs` (`Content-Length`
  framing, 64 MiB cap), `text.rs` (UTF-16 positions, incremental changes, language by id or
  extension, `file:` URIs), `files.rs` (a TMDL file's `definition` folder, read from disk with a
  size and mtime cache, capped at 10,000 files and 256 MiB), `mod.rs` (lifecycle, document sync,
  the model from `--model` or `initializationOptions`, reopened when a file under it is saved;
  diagnostics pushed once the incoming queue is empty; `$/cancelRequest` even before the request
  arrives) and `features.rs` (completion, hover, signature help, definition, references,
  highlights, formatting, quick fixes, semantic tokens). Every answer comes from a registry
  command (`dax.*`, `m.*`, `tmdl.*`, `doc.open`) run through `Registry::execute`, so no new
  command and no new crate edge. "Did you mean" suggestions become quick fixes: DAX's are mapped
  onto the flagged range (a bracketed column or measure, a quoted or plain table); M's and TMDL's
  carry their replacement. TMDL formatting keeps Desktop's tabs unless the file already uses
  spaces. Setup for Neovim, Helix and VS Code in `docs/CLI.md`.
- **Why:** M7.6 (DESKTOP-PLAN), the last M7 step: the language services of M7.1–M7.4 in
  external editors. M7 is complete apart from A6.
- **Desktop check:** the server adds no language logic; its diagnostics, completion and
  formatting are the commands checked against Power BI Desktop (DAX arity, M library and
  messages, the 2,507 DAX queries) and Microsoft's TMDL reader and writer in M7.1–M7.4. A manual
  `Fuzz` run on `main` covers the `dax_service`, `m_service` and `tmdl_service` targets
  (acceptance 4).
- **Numbers:** 7 protocol tests with a scripted client (`tests/lsp.rs`: lifecycle and errors
  `-32002`, `-32600`, `-32601`, `-32700`, `-32800`; DAX, M and TMDL features with exact ranges;
  a TMDL folder on disk with a cross-file definition and diagnostics; names from a model opened by
  `initializationOptions` and again on save; Acme from `--model`; exit codes), 13 unit tests;
  dashcraft-cli lib tests 7 → 20. About 2,450 lines of Rust (including 706 of tests). The 7
  protocol tests take 0.1 s in a debug build.
- **Open:** A6 for M7 (a person); DAX-only navigation and tokens (M has no references or tokens
  service yet); no VS Code extension is shipped.
- **Commit:** `3f872e3` (PR #64).

### M7.5b Formula editor keys and accessibility

- **What:** the formula bar's DAX editor gets the keys of Microsoft's table for Power BI Desktop's
  formula editor that one caret can do: Tab and Shift+Tab, Ctrl+] and Ctrl+[ indent and outdent;
  Ctrl+M makes Tab move focus (shared by every formula editor in the window, in the editor's
  accessible description and announced); Shift+Enter breaks the line keeping its indentation and
  Ctrl+Shift+Enter inserts one above; Alt+↑ ↓ move, Shift+Alt+↑ ↓ copy and Ctrl+Shift+K
  delete the lines; Ctrl+L selects the line (again: the next); Ctrl+X and Ctrl+C without a
  selection take the line (the line is selected on keydown so the browser's own cut and copy run,
  and the clipboard handler writes the line); Ctrl+Shift+\\ jumps to the matching bracket
  (strings and comments skipped); Shift+Alt+→ ← expand and shrink the selection (word,
  bracket contents, brackets, line, all); Ctrl+↑ ↓ and Alt+PgUp PgDn scroll. Pure helpers in
  `app/src/lib/daxEditor.ts`, each edit applied with `insertText` so the editor's undo keeps it.
  `scripts/app-checks/formula-keys.mjs` runs the keys and A1–A5 with the suggestion list,
  parameter info, a description and a squiggle open; the checker helpers moved from
  `editing-a11y.mjs` to a shared `a11y.mjs`. Results in `docs/accessibility/m7-formula-bar.md`.
- **Why:** M7.5 (DESKTOP-PLAN), its acceptance criterion 5 (A1–A6 for the formula bar with
  completion lists and squiggles); keys from Microsoft's *Formula editor in Power BI Desktop*.
- **Numbers:** app node tests 35 → 45 (`daxEditor.test.ts` 15 → 25); `formula-keys.mjs` 43
  of 43, `formula-intellisense.mjs` 20 of 20, `editing-a11y.mjs` 17 of 17 and `editing-smoke.mjs`
  23 of 23 on Windows / WebView2. Contrast: suggestions at least 5.4:1, the selected
  suggestion's outline 5.39:1, squiggles 5.86:1 (error) and 5.73:1 (warning), parameter info
  6.19:1, descriptions 6.07:1; focus ring 2 px at 5.21:1. Main chunk 445 kB → 453 kB. Found by the tests: moving the last
  lines down left the selection past the end of the text.
- **Commit:** `f7afc32` (PR #63).
- **Open:** A6 (screen-reader runs) needs a person; extra cursors, column selection, Ctrl+G and
  Ctrl+U are not supported; M7.6 (`dashcraft lsp`).

### M7.5a Formula bar IntelliSense

- **What:** a reusable DAX editor, `app/src/components/DaxEditor.svelte`, now in the formula bar
  (`FormulaBar.svelte`) for measures, calculated columns and calculated tables. A transparent
  textarea over a coloured overlay keeps native editing, selection, IME and undo; the overlay
  draws `dax.tokens` colours and `dax.diagnostics` squiggles, realigned while typing until the
  next answer (150 ms debounce, late answers dropped). A suggestion list from `dax.complete`
  opens on a name, `[` or `'` and on Ctrl+Space (↑ ↓ PgUp PgDn, Enter or Tab insert without
  doubling a function's `(`, Esc closes; a listbox with `aria-activedescendant`, a side panel with
  the item's description and a polite status); parameter info from `dax.signatureHelp` after `(`
  and `,` and on Ctrl+Shift+Space (active parameter bold); descriptions from `dax.hover` and the
  problems under the pointer or on Ctrl+K Ctrl+I; Shift+Alt+F formats the expression after
  `Name =` with `dax.format`. The editor's accessible description carries the problem at the
  caret, else the first error. Pure helpers in `app/src/lib/daxEditor.ts` (overlay segments, range
  shifting, caret geometry, completion edits, keys); `backend.daxService` sends the requests for
  the open document. The bundle's dependencies go to a `vendor` chunk (Vite's 500 kB warning).
  `editing-a11y.mjs` now measures the contrast of the coloured overlay, since the textarea's
  own text is transparent.
- **Why:** M7.5 (DESKTOP-PLAN), the editor the M7.1 and M7.2 commands were built for; the keys
  follow Microsoft's documentation of Power BI Desktop's formula editor. The calculation item,
  format string and RLS editors (M10, M11) reuse the component.
- **Numbers:** app node tests 19 → 35 (`daxEditor.test.ts` 15, one in `editing.test.ts`; token
  colours at least 4.5:1 on the editor background); `formula-intellisense.mjs` 20 of 20 checks,
  `editing-smoke.mjs` 23 of 23 and `editing-a11y.mjs` 17 of 17 (lowest text contrast 5.98:1) on
  Windows / WebView2. Main chunk 486.65 kB → 445 kB plus a 64 kB vendor chunk. Found live: the
  input handler ran before Svelte's `bind:value` updated, so requests used the previous text; the
  editor now reads the textarea itself.
- **Commit:** `3d4ff95` (PR #62).
- **Open:** M7.5b (Desktop's line-editing keys, Ctrl+M, A1–A6 for the completion list and
  squiggles), M7.6 (`dashcraft lsp`).

### M7.4c TMDL formatter

- **What:** `pbi_tmdl::service::format` (`service/format.rs`) with `FormatOptions` (tabs or
  1–16 spaces, kept or chosen line ending) and `FormatError::{Syntax, Unstable}`; the service
  re-exports `IndentStyle` and `LineEnding`. It re-emits the CST: each structural line indented
  by its depth and spaced as the writer spaces it, indented expression blocks two levels below
  their line (one for a shallow nameless property, as Desktop writes KPI expressions) with the
  common prefix replaced and relative indentation and inner trailing white space kept, fenced
  blocks moved with their closing fence, comments and `///` lines at the node's level. Blank
  lines: one before named objects and the first `ref` line, none before `keyword: value`
  properties, flags and same-type `ref` lines; kept as written before nameless objects, `ref`
  runs of another type and perspective members, where Desktop and TOM disagree or vary. The
  output is parsed again and its shape (keywords, names, operators, values, expressions,
  descriptions, comments, children) must equal the input's, else `Unstable`. New command
  `tmdl.format` (`indentWidth`, `lineEnding`; `ok: false` with the TMDL110 diagnostic on a
  syntax error). The `tmdl_service` fuzz target and the mutation test also format (idempotent;
  the mutation test also requires the same model).
- **Why:** M7.4 (DESKTOP-PLAN): "formatting that matches the writer"; the TMDL view and external
  editors need a formatter that leaves Desktop's files alone.
- **Numbers:** 88 → 89 registry commands. Ground truth: all 58 files saved by Desktop and the
  TOM library (Acme, `tmdl-probe`, dax-lab, GT-4 s24 before and after, CRLF kept) and all 53
  files `write_model` writes for those models format to themselves; each of the 58 with its
  layout damaged (every structural line respaced with trailing white space, blank lines doubled
  or removed, deep blocks indented two more levels, LF for CRLF) formats back to the saved
  bytes, and each goes to four spaces (stable) and back to the original. The first run found
  the two places where TOM and Desktop differ (blank lines between `ref` types, before
  perspective members). Mutation test (3,000 cases): 1,733 of 1,735 parsing files formatted, the
  2 others declined (a header value continued by a block as the first indented line, which
  space indentation cannot express); 20,000 release cases (seed 11): 11,666 of 11,686 formatted,
  20 declined, no other failure. Tests: `format.rs` 7 unit
  tests, `groundtruth.rs` 9 (3 new), `tmdl_language.rs` 10 (3 new).
- **Commit:** `d414523` (PR #61).
- **Open:** M7.5 (formula bar IntelliSense), M7.6 (`dashcraft lsp`).

### M7.4b TMDL completion and hover

- **What:** `pbi_tmdl::service::{complete, hover}` (`service/complete.rs`) over a tolerant
  outline of the text (`service/outline.rs`: structural lines, parents and expression blocks of
  half-typed TMDL, and what the cursor is on: a keyword, a value, a name, a `ref` object type).
  The other files of the folder are passed as `FileText`s: their tables (with columns,
  measures, hierarchies and whether a partition is calculated), other objects and `database`
  compatibility level are indexed as the diagnostics index them. Completion: the keywords of
  the line's context from the catalog, at the compatibility level, without properties already
  set by a sibling, properties after a child object, objects before a property, or a second
  `model`/`database`, plus `ref` and the opaque `translations`; values of `bool` and enum
  properties; references (`sortByColumn`, `fromColumn`, `defaultHierarchy`, `queryGroup`,
  `tablePermission`, `perspectiveColumn`, `ref table`, …) resolved as `check_reference`
  resolves them; ranked exact, prefix, word start, contains. Hover: keyword forms, value type
  and level against the database's; enum and boolean values; a referenced object's `///`
  description, declaration line and file. New commands `tmdl.complete` and `tmdl.hover`
  (`path` + `files` for the folder, positions in UTF-8, UTF-16 or line/column). The fuzz
  target `tmdl_service` and the robustness mutation test also complete and hover at several
  cursors.
- **Why:** M7.4 (DESKTOP-PLAN): the TMDL view and external editors need completion of
  properties and object references.
- **Numbers:** 86 → 88 registry commands. Recall over the Acme, `tmdl-probe`, dax-lab and GT-4
  models (written by Desktop and TOM): 1,936 of 1,936 keywords, 497 of 497 values and 48 of 48
  references offered at their end and hovered; precision: 841 completions written into the
  first and last child position of each kind of object in Acme and dax-lab, in every offered
  form, give no keyword error from the diagnostics; both tests take about 6 s (debug). Found and
  fixed on the way: `ref culture` (TOM's spelling) was not offered, a second `model` or
  `database` was, and booleans had no hover. Tests: `complete.rs` 8 unit tests, `outline.rs` 3,
  `groundtruth.rs` 6 (2 new), `tmdl_language.rs` 7 (3 new); a 20,000-case mutation run found no
  completion or hover outside its file.
- **Commit:** `387188e` (PR #60).
- **Open:** M7.4c (`tmdl.format`), M7.5, M7.6.

### M7.4a TMDL diagnostics

- **What:** `pbi_tmdl::service` (new module): `catalog` (`catalog.txt`, generated from the
  recording: per context the keywords, their forms, value types and compatibility levels, and
  the names and integers of each enumeration) and `diagnostics` over a set of `Source` files
  (codes TMDL101–113 errors, TMDL120 warning, plus the parser's TMDL006/007), with "did you mean"
  suggestions; references are resolved when the files form a model folder, compatibility levels
  when a `database` declares one. New registry command `tmdl.diagnostics` (category `tmdl`; UTF-8,
  UTF-16 and line/column positions). The oracle is Microsoft's TMDL reader (TOM's
  `TmdlSerializer` from NuGet, black box; ADR-0029): `scripts/groundtruth/Record-Tmdl.ps1`
  records the corpora `tmdl-catalog` (73 cases over the new `reference/groundtruth/tmdl-probe`
  model) and `tmdl-validity` (54 edits of Acme), generated by `gen_corpus.py`;
  `cargo xtask groundtruth` lists them. `crates/pbi-tmdl/tests/groundtruth.rs` regenerates the
  catalog and compares every verdict. New fuzz target `tmdl_service`; the mutation test of
  `crates/pbi-tmdl/tests/robustness.rs` also checks the diagnostics.
- **Why:** M7.4 (DESKTOP-PLAN): the TMDL view and external editors need to underline what Desktop
  will refuse to open, before the model is saved.
- **Numbers:** 85 → 86 registry commands; 16 → 17 fuzz targets. Catalog: 43 contexts, 160
  keywords, 30 enumerations. 4,106 of 4,109 catalog checks match the reader (3 ledgered: a
  `bindingInfo` without `type`, twice, is lenient; an empty `translations` block, which crashes
  the reader, is by design); 78 of 78 `tmdl-validity` checks match (verdict and line); no error
  on the Acme, `tmdl-probe`, dax-lab and GT-4 models. The whole comparison takes about 0.5 s.
  Tests: `diagnostics.rs` 7 unit tests, `groundtruth.rs` 4, `tmdl_language.rs` 4 (pbi-ops); a
  20,000-case mutation run of the robustness test found no diagnostic outside its file.
- **Open:** M7.4b (completion and hover from the catalog), M7.4c (`tmdl.format`), M7.5, M7.6.
- **Commit:** `ebbf24e` (PR #59).

### M7.3d M formatter

- **What:** `pbi_m::service::format` (`service/format.rs`) and the registry command `m.format`
  (category `m`; `text`, `maxWidth` default 100, `indentWidth` default 4 up to 16, `useTabs`;
  returns `ok`, `text`, `changed` and a syntax `diagnostic`). Comments attach to the tokens
  they precede or follow; each construct becomes a group that breaks only when it does not fit
  (Wadler-style): one `let` step per line with blank lines between steps kept, operators leading
  continuation lines, `else if` chains at one level, `try … otherwise`/`catch`, records, lists,
  calls, record and table types, function bodies on the next line. Generalized identifiers and
  literals are kept verbatim. The output is re-parsed and its tree (spans cleared), tokens and
  comments must equal the input's, else the call fails rather than change the query. The fuzz
  target `m_service` also formats with the default options and with options from its first byte.
- **Why:** M7.3d (DESKTOP-PLAN), M7 acceptance 1 for M, GAPS §2 language service. Closes M7.3.
- **Numbers:** 84 → 85 registry commands. 6 unit tests; 42 of the 44 queries of Acme and the
  samples change at the default width, and all 44 are stable at width 100 and at width 40 with
  2-space indents (`crates/pbi-session/tests/m_service.rs`); registry tests 6 → 8
  (`crates/pbi-ops/tests/m_language.rs`); the 268 `m-functions` expressions formatted at the
  default width and at width 20 with tabs give Desktop's recorded results (267 match, the same 1
  by-design ledger entry; unknown-name errors compared without their moved line and column). A
  replay of the fuzz target (about 3 million byte mutations of the seeds, then 320,000 comment
  and whitespace insertions into 17 parseable seeds including the Acme queries) found no unstable
  or non-idempotent output. Seeds 12 → 14.
- **Commit:** `9d5e438` (PR #58).
- **Open:** M7.4 (TMDL language service), M7.5 (formula bar), M7.6 (`dashcraft lsp`); semantic
  tokens and references for M. A line comment between `)` and the `,` on the next line leaves
  the comma on its own line (stable, cosmetic).

### M7.3c M hover, signature help and completion

- **What:** `pbi-m` gains a tolerant lexer (`tokenize_tolerant`: an unterminated text, quoted
  identifier or comment runs to the end of the text) and a scanner (`service/scan.rs`) that reads incomplete
  text: the scopes visible at a cursor (let steps, record fields, function parameters, `each`'s
  `_`, section members, `@` self-references; Desktop's scoping), the call and argument around it
  and the kind of position (declaration, field name, type after `as`/`is`, field type of a record
  or table type, expression). `service/help.rs` gives hover (steps, parameters, model names
  with their kind, type and description, library functions with their signature, types, constants,
  unsupported functions) and signature help (label, UTF-8 parameter ranges, active parameter);
  `service/complete.rs` ranks steps and locals, model names, keywords and the library by match and
  by kind, quotes names that need `#"…"`, completes inside `#"…` and after `@`, and offers
  primitive types after `as` and any expression in a field type (`type table [X = Int64.Type]`).
  `Environment` entries carry the kind, M text, type and description of each model name
  (`pbi_session::m_environment`: queries, parameters with their type, functions, tables). New
  registry commands `m.hover`, `m.signatureHelp` and `m.complete` (category `m`, UTF-8, UTF-16 or
  line and column positions, with or without a document). New fuzz target `m_service`.
- **Why:** M7.3c (DESKTOP-PLAN), GAPS §2 language service: the M editor needs help at the cursor.
- **Numbers:** 81 → 84 registry commands; 15 → 16 fuzz targets. Over Acme and the four committed
  samples, every one of 418 free names hovers to a model or library name and completes first at
  its end, and signature help names each of 285 library calls (`crates/pbi-session/tests/m_service.rs`);
  no panic at any cursor of the Acme queries. Unit tests: `scan.rs` 6, `help.rs` 4,
  `complete.rs` 6, `names.rs` 3, `environment.rs` 1, `lexer.rs` +2; registry tests in
  `crates/pbi-ops/tests/m_language.rs` 3 → 6. A mutation replay of the fuzz target found an
  empty completion label for the valid step name `#""` (fixed, seed added). GAPS §2 language
  service 2 → 3.
- **Commit:** `657df86` (PR #57).
- **Open:** M7.3d (`m.format`), semantic tokens and references for M.

### M7.3b M diagnostics and Desktop's name binding

- **What:** `pbi-m` binds names when a query is compiled, as Power BI Desktop does: `refs.rs` scopes
  let and record initializers exclusively (a step does not see itself), lists calls of free names
  and the free names of a section; the new `binding.rs` decides what is unknown (library names,
  custom-connector names) and holds Desktop's messages; `eval_source` and shared-query compilation
  reject an unknown name before evaluating anything, and the `SelfOnly` lookup fallback is gone.
  `Expression.Evaluate` joins the library; `#shared` lists the whole catalog; the arity error drops
  the function name. `pbi_m::service::diagnostics` (syntax, `unknownName` with a suggestion,
  `argumentCount`, `unsupported` and `extensionName` warnings, an `Environment` of model names)
  backs the new `m.diagnostics` command (category `m`; `pbi_session::{m_service, m_environment}`).
- **Why:** M7.3b (DESKTOP-PLAN), GAPS §2: the M editor needs diagnostics, and the evaluator was
  more lenient than Desktop (it ran queries Desktop refuses).
- **Desktop check:** partition queries of a temporary table, refreshed through TMSL, showed that an
  unknown name anywhere (unused step, `try`, function body, untaken branch, type) fails the whole
  query with "The name 'X' wasn't recognized.  Make sure it's spelled correctly." (two spaces),
  that a misspelled library name gives the same message, and that the arity message never names
  the function. The M recorder evaluates cases with `Expression.Evaluate(case, Library)`, whose
  message differs ("[l,c-l,c] The name 'X' doesn't exist in the current context."); the M
  ground-truth test now evaluates its cases the same way. 29 new `m-functions` cases (`names/*`,
  `arity/*`) and the suite `m-arity` (`scripts/groundtruth/m-arity.m`: one argument too few and
  too many for each of the 941 functions).
- **Numbers:** `m-functions` 239 → 268 expressions, 267 match, 1 by design; `m-arity`: 941
  functions, the language service's messages all match and so do the evaluator's 761 calls of
  implemented functions; `m.diagnostics` finds no false error in the 44 queries of Acme and the
  committed samples; 6 unit tests in `service/diagnostics.rs`, 3 in `refs.rs`, 3 registry tests in
  `crates/pbi-ops/tests/m_language.rs`.
- **Commit:** `e4b7e72` (PR #56).
- **Open:** M7.3c (`m.hover`, `m.signatureHelp`, `m.complete`), M7.3d (`m.format`).

### M7.3 M library growth, part 2

- **What:** `pbi-m` gains 49 `Table.*` functions in two new modules. `stdlib/table_ops.rs` has the
  reshaping functions (joins with every `JoinKind`, `NestedJoin`, `AddJoinColumn`, `Group` with
  `GroupKind.Local` and comparers, `Pivot`, `Unpivot`, `UnpivotOtherColumns`, the three expansions,
  `AggregateTableColumn`, `SplitColumn`, `CombineColumns`, `CombineColumnsToRecord`,
  `Transpose`); `stdlib/table_rows.rs` has the row functions (`Max`/`Min`/`MaxN`/`MinN`,
  `AddRankColumn`, `Schema`, `ColumnsOfType`, `FromValue`, `ToList`, `Partition`, `Split`,
  `Repeat`, `AlternateRows`, row insertion, removal and replacement, `Contains*`, `PositionOf*`,
  `*MatchingRows`, `FindText`, …). `table.rs` shares its helpers and sorts with a panic-free stable
  merge sort (an inconsistent comparer cannot panic it). Conversion errors take Desktop's wording
  for structured values ("a value of type List", "We cannot convert Type to Table type").
- **Why:** M7.3 (DESKTOP-PLAN), GAPS §2 "Standard library": the 10 table functions ledgered in
  part 1 and the rest of the commonly generated table steps.
- **Desktop check:** 37 new `m-functions` cases, recorded in several rounds against Power BI Desktop
  2.159, settled the edge cases: column placement after unpivot and combine (the last listed
  column's place), pivot types (the value column's, nullable), `ExtraValues.List`, the ordering of
  `AddRankColumn` and of ties in `Max`/`Min`, `Number.Mod` semantics in `Partition` hashes,
  `Contains` (record fields only) versus `PositionOf` (whole row), out-of-range argument messages,
  and `Table.FindText` looking into lists and records. A record missing a column makes its row an
  error rather than the table (`InsertRows`, `ReplaceMatchingRows`, `FromRecords`); such errors
  escaped `try` in Desktop until the corpus read every cell through `x{i}` inside its own `try`.
  Desktop's join row order is not specified; the corpus sorts join results.
- **Numbers:** library 461 → 514 names (335 → 384 functions, 49 → 50 types, 77 → 80 constants;
  `Table.*` functions 45 → 94 of Desktop's 114); `m-functions` 202 → 239 expressions, 238 match,
  1 by design; ledger 12 → 2 entries (one M, one DAX format); 6 new tests in `stdlib.rs`.
- **Commit:** `210c667` (PR #55).
- **Open:** the 20 `Table.*` functions left out (fuzzy matching, keys, partition keys,
  `Table.View`, `Table.Profile`, connector internals); M7.3b–d (M diagnostics, hover, completion,
  formatting).

### M7.3 M library growth, part 1

- **What:** `pbi-m`'s library gains 69 functions, the missing enum constants and 18 enum `*.Type`
  values. `stdlib/text.rs`: seven `Splitter.SplitTextBy*` functions, `Text.NewGuid` (version-4
  GUIDs from a generator seeded by the evaluation clock, so a `FixedClock` repeats them),
  `Text.PositionOfAny`; `number.rs`: `Number.Bitwise*`; `list.rs`: statistics (`Median`, `Mode`,
  `Modes`, `StandardDeviation`, `Covariance`, `Product`, `Percentile` with `PercentileMode`) and
  editing (`Split`, `Alternate`, `InsertRange`, `ReplaceRange`, `RemoveRange`, `MaxN`, `MinN`,
  `TransformMany`, …); `datetime.rs`: week, quarter, day and hour boundaries, `AddWeeks`,
  `AddQuarters`, `*.ToRecord`, `MonthName`/`DayOfWeekName` (names for 18 languages from Desktop in
  the new `date_names.rs`), Desktop's 0001–9999 date range and its overflow and `Day` errors;
  `record.rs`: `TransformFields`, `ReorderFields`; new `binary.rs`: `Binary.*`, `BinaryEncoding`,
  `Json.FromValue` (Desktop's escaping, durations as `P1DT2H3M4S`, NaN as `null`).
- **Why:** M7.3 (DESKTOP-PLAN), GAPS §2 "Standard library": the ledgered functions of M7.3a.
- **Desktop check:** 11 probe batches against Power BI Desktop 2.159 settled the edge cases
  (splitter quoting, percentile interpolation, week numbering, culture names, overflow messages,
  strict base64, JSON escaping). Suite `m-functions` grows from 177 to 202 expressions: 191 match,
  10 table functions ledgered `unsupported` (part 2), 1 by design. Desktop evaluates `Binary` values
  lazily, so an error inside `try Binary.FromList(…)` escapes the `try`; the corpus forces such
  values with `Binary.Length`.
- **Numbers:** library 351 → 461 names (266 → 335 functions, 31 → 49 types, 54 → 77 constants);
  ledger 29 → 12 entries; new tests in `stdlib.rs` (GUIDs, date range, JSON, base64) and
  `binary.rs`.
- **Commit:** `dfad70a` (PR #54).
- **Open:** part 2, the table functions (`Table.Group`, `Pivot`, `Unpivot`, `Join`, `NestedJoin`,
  `Transpose`, `SplitColumn`, `Max`, `Schema`, expansions).

### M7.3a M library catalog and Desktop conformance

- **What:** the ground-truth recorder evaluates M: a corpus with `"language": "m"` runs each
  expression in Desktop's mashup engine through a temporary hidden table
  (`Invoke-GtMCases`, an M probe that encodes values as typed JSON, in
  `scripts/groundtruth/GroundTruth.psm1`). Two suites: `m-library` (`scripts/groundtruth/m-library.m`:
  every dotted name of `#shared` with kind, parameters, return type, type or constant value) and
  `m-functions` (177 expressions from `gen_corpus.py`). `pbi_m::service::{catalog, catalog_entry}`
  read `src/service/library.txt`, generated from the `m-library` recording by
  `tests/library.rs`, which also checks every implemented name against it. `tests/groundtruth.rs`
  compares the `m-functions` recording with DashCraft (the DAX side's ledger, report and rules).
  `value/digits.rs` (decimal digit buffers) gives .NET's number text (R and G layouts, E, F, N, P,
  D, X and custom formats) and decimal rounding to `Text.From`, `Number.ToText`, `Number.Round*`
  and error messages. Fixes: error records (`Message.Format` placeholders, `ErrorCode`), splitter
  quoting (`csvStyle`, `startAtEnd`), `#datetimezone` offsets, text ranges, parameter names,
  `Number.MaxValue`/`MinValue` removed (not in Desktop). `pbi-ops`' DAX-language test skips M
  corpora.
- **Why:** M7.3 (DESKTOP-PLAN): M hover, signature help and completion need Desktop's library, and
  ADR-0011's ground truth did not cover M yet (GAPS §2).
- **Desktop check:** Power BI Desktop 2.159. Library: 1,142 names (941 functions, 69 types, 132
  constants and values); DashCraft's 351 names (266 functions) all match their kind, parameter
  names and counts, types and constants. `m-functions`: the first comparison had 36 mismatches;
  now 149 of 177 match, 27 call functions DashCraft lacks
  (ledgered `unsupported`), 1 by design (Desktop keeps number literals as decimals until
  arithmetic). Desktop probes settled the number rules: G15 when it round-trips else G17, numbers
  below 7.9e28 written out from 15 digits, `E` with a signed 3-digit exponent, F/N/P/custom
  rounded half away from 15 digits, `Number.Round` in decimal with `RoundingMode` breaking ties
  only.
- **Numbers:** new tests: `library.rs` 3, `groundtruth.rs` 3, `digits.rs` 2, regression cases in
  `stdlib.rs` and `eval.rs`; GAPS §2 3.1 → 3.3. Ground truth: 14 suites, 3,366 cases, 29 ledger
  entries.
- **Commit:** `5868808` (PR #53).
- **Open:** the 27 ledgered functions (`Table.Group`, `Table.Pivot`, `Table.Join`,
  `Splitter.SplitTextBy*`, `List.Median`, `Date.WeekOfYear`, …), then M7.3b–d (diagnostics, hover
  and completion, formatter).

### M7.2 DAX formatter

- **What:** `pbi_dax::service::format` (`formatter.rs`) with `FormatOptions` (line width, indent
  width or tabs, keyword and built-in function casing) and the `dax.format` command. Comments are
  attached to the nearest syntax node and kept, single blank lines between `VAR`s and `DEFINE`s
  are kept, user names and literals keep their spelling. The layout is a group-based pretty
  printer: a call, table constructor or operator chain stays on one line when it fits, else
  breaks one argument per line or before each operator, nested by precedence. A safety net
  re-tokenizes and re-parses the output: if its tokens or syntax tree differ from the input's,
  `format` returns `Unstable` (the command reports an internal error) instead of text. The
  generator of `roundtrip.rs` moved to `tests/common/generate.rs` to be shared. The `dax_service`
  fuzz target also formats every input with the defaults and with a narrow width taken from the
  first byte, checking for no `Unstable` and idempotence (new seed `format_comments.dax`).
- **Why:** M7.2 (DESKTOP-PLAN), the formatter the DAX query view and the formula bar (M7.5) need;
  acceptance 1 for DAX.
- **Desktop check:** the 2,902 queries of the `acme-*`, `format`, `dax-arity`, `dax-functions`
  and `dax-lab-tables` corpora, formatted at width 40 with tabs and lower-case keywords and
  functions (all but 2 syntax-error probes change), were run on Power BI Desktop 2.159 next to the
  originals on the same models: 2,902 of 2,902 give the same result (2,180 identical values, 722
  errors in both). Not committed as recordings (they repeat the corpora).
- **Numbers:** `format.rs` 19 tests (pbi-dax): layout cases, comments, casing, indentation, CRLF,
  syntax errors, and properties (idempotent, same significant tokens, same tree, comments kept,
  stable under four widths, tabs and casings) over every test-data file, 1,500 generated
  expressions and 300 queries with injected comments, and token soup. `dax_language.rs` 9 → 10
  (pbi-ops): all 136 Acme and sample expressions and all 2,507 queries Desktop ran without error
  format idempotently with no errors after. 79 → 80 registry commands.
- **Commit:** `e56319a` (PR #52).
- **Open:** M7.3 (M language service and formatter).

### M7.1d DAX semantic tokens and references

- **What:** `pbi_dax::service::{semantic_tokens, references}` (`symbols.rs`) over a scope analysis
  shared with completion (`scope.rs`: `VAR`/`RETURN` scopes, function parameters, query
  `DEFINE`s). Tokens carry 14 kinds, a declaration flag and an unresolved flag (never without a
  model); references find the symbol at a cursor (variable, parameter, query definition, table,
  column, measure, function) and every occurrence, named as the model spells it. The registry
  gains `dax.tokens` and `dax.references` (positions as UTF-8, UTF-16 and line/column through an
  incremental mapper; the model object's `objectId`). The `dax_service` fuzz target also runs
  both (tokens ordered and disjoint, the cursor's name among the references, the definition a
  declaring site).
- **Why:** M7.1d (DESKTOP-PLAN), the highlighting, go to definition and find references the
  formula bar (M7.5) and `dashcraft lsp` (M7.6) need.
- **Desktop check:** a new test runs `dax.diagnostics` and `dax.tokens` on every query of the
  ground-truth corpora that Power BI Desktop ran without error, against its fixture model: 2,507
  queries. It found 32 cases: `INFO.*` functions and `NAMEOF` components shown as unknown, and a
  false `unknownMeasure` error on `FILTER(INFO.VIEW.COLUMNS(), [Type] <> ...)`. Fixed: `INFO.*`
  names are functions, `NAMEOF`'s component is a keyword, and a bare `[name]` is not flagged once
  the text calls an `INFO` or user-defined function (whose columns are unknown). Now 0 of 2,507.
- **Numbers:** tests `language_service.rs` 25 → 28 (pbi-dax), `dax_language.rs` 7 → 9 and one
  unit test (pbi-ops). 77 → 79 registry commands. Commit `dc01508` (PR #51).
- **Open:** M7.2 (formatter), M7.3 (M language service).

### M7.1c DAX completion

- **What:** `pbi_dax::service::complete` (`complete.rs`): completion at a cursor from the tokens
  around it, so it works while the text does not parse. The context decides what fits: an
  expression start (functions, tables, qualified columns, measures, variables in scope, `VAR`,
  `RETURN`, `DEFINE`/`EVALUATE` at the start of a query), after an operand (`IN`, `VAR`, `RETURN`
  and the query keywords), inside `[` (the table's or every column and measure), inside `'`
  (tables and qualified columns), after `DEFINE`, `ORDER`, `START`, query `MEASURE`/`COLUMN`;
  nothing in comments, literals or after `VAR`. Variables follow `VAR`/`RETURN` scopes and
  parentheses; a query's `DEFINE`s count. Ranking: exact, prefix, word start, contains; the active
  parameter's kind (`table`, `column`) first; then variables, functions and keywords, tables,
  columns, measures. The registry gains `dax.complete` (`maxItems` default 100, at most 1,000,
  `incomplete` when cut). P12 `dax-complete` (`perf_dax.rs`, `perf/budgets.toml`). Fuzz target
  `dax_service` (diagnostics, signature help, hover and completion at every offset; 20 seeds from
  the `dax_parse` seeds and the service tests).
- **Why:** M7.1c (DESKTOP-PLAN), the completion list the formula bar (M7.5) needs; acceptance 3
  (completion p95 within P12's target) and 4 (a language-service fuzz target in the nightly job).
- **Numbers:** P12 3.0 ms p95 (median 1.5 ms) over 20 tables × 20 columns and 2,000 measures on a
  12-core Windows desktop (target 20, max 50). The fuzz target ran 72,362 mutated inputs through a
  stable-toolchain harness without a finding (nightly runs it under libFuzzer). Tests:
  `language_service.rs` 18 → 25 (pbi-dax; one runs every service at every offset of awkward
  texts), `dax_language.rs` 6 → 7 (pbi-ops). 76 → 77 registry commands; 14 → 15 fuzz targets.
  Commit `1b77856` (PR #50).
- **Open:** M7.1d (semantic tokens, go to definition, find references), M7.2 (formatter).

### M7.1b DAX function catalog, argument counts, signature help and hover

- **What:** `pbi_dax::service` gains a function catalog (`catalog.txt`, parsed once by
  `catalog.rs`: signature, parameters, argument count, return type, category and a DashCraft-written
  description for each of the 299 functions), argument-count diagnostics (`argumentCount`),
  `signature_help` and `hover` (functions, user-defined functions, tables, columns, measures with
  their expression and format string, variables with their expression, keywords; the `DEFINE`s of
  a query count). Both work on tokens, so they answer on text that does not parse yet. The
  registry gains `dax.signatureHelp` and `dax.hover` (cursor as UTF-8 or UTF-16 offset or line and
  column); `dax.diagnostics` and `dax.hover` share the document schema, which now carries the
  report-level measures' details. Ground truth gains a `dax-arity` suite (`gen_corpus.py`): each
  catalog function called with too few, the fewest, the most and too many arguments; suites can
  set `recordValues: false` (`Record-GroundTruth.ps1` then records only columns, row count or
  error).
- **Why:** M7.1b (DESKTOP-PLAN), the next language-service step for the formula bar (M7.5).
  Arity errors are only worth showing when they match Desktop, so the catalog's counts are
  checked against it rather than written from memory.
- **Numbers:** 929 arity probes recorded from Power BI Desktop 2.159: 285 accepted, 530 rejected
  with an argument-count message, 114 inconclusive (another error first); DashCraft's catalog and
  `diagnostics()` agree on all 815 conclusive ones, and no recorded query of any suite gets an
  `argumentCount` error. Desktop's answers set counts the signatures alone got wrong
  (`USERNAME` takes any number of arguments, `WINDOW` up to 10). Tests: `language_service.rs`
  10 → 18 (pbi-dax), 6 catalog unit tests, `catalog_argument_counts_accept_every_tested_expression`
  (every `functions.rs` expression), `dax_language.rs` 3 → 6 (pbi-ops). Ground truth: 2,259 →
  3,188 recorded queries. 74 → 76 registry commands. Commit `70e77c5` (PR #49).
- **Open:** M7.1c (completion, P12, a language-service fuzz target), M7.1d (semantic tokens, go to
  definition, find references).

### M7.1a DAX diagnostics and `NAMEOF`

- **What:** `pbi-dax` gains a language service (`pbi_dax::service`): `Schema` (tables, columns,
  measures and user-defined functions from a model, or `Schema::unknown()` for syntax and
  function checks only) and `diagnostics` (syntax errors, unknown tables, columns and measures with
  "did you mean" suggestions, functions DashCraft cannot evaluate as warnings; sorted, capped at
  1000). The registry command `dax.diagnostics` (`pbi-ops`, through `pbi_session::dax_service`)
  checks a text against a document's model and report-level measures, with UTF-8, UTF-16 and
  line/column positions. The name matching of the linter's suggestions moved to `pbi-model`
  (`closest_name`) and is shared. The evaluator gains `NAMEOF` (components `TABLE`, `COLUMN`,
  `MEASURE`, `FULL`, `SELF`, `PARENT`, Desktop's quoting and error messages).
- **Why:** M7.1 (DESKTOP-PLAN), the first step of the language foundation; acceptance 2 asks for
  no false errors on every Acme and sample expression. The only warning left there was `NAMEOF`
  in Corporate Spend's field parameters, so it was implemented rather than accepted.
- **Numbers:** 136 DAX expressions (Acme 54, Corporate Spend 23, COVID-19 US Tracking 28, Regional
  Sales 23, Revenue Opportunities 8): no errors, no warnings. Tests: `language_service.rs` (10,
  pbi-dax), `dax_language.rs` (3, pbi-ops), `nameof_quoting_query_tables_and_errors` and 17
  `NAMEOF` cases in `functions.rs`. Ground truth: `dax-functions` 552 → 569 cases, all equal to
  Power BI Desktop 2.159 (2,259 recorded queries). 298 → 299 DAX functions (24 partial); 73 → 74
  registry commands.
- **Open:** M7.1b (completion, signature help, hover, arity checks, P12), M7.1c (tokens,
  references), a fuzz target for diagnostics (acceptance 4).
- **Commits:** `6185a50`, `d8339c6` (PR #48).

### M6 complete (docs)

- **What:** M6 ticked in the ROADMAP; the plan, ROADMAP and GAPS say where the last open items
  went: deleting a data column edits its query's M and comes with the Power Query editor (M14.7,
  M14.9), editing a hierarchy's levels with the hierarchy editor (M10.5). The screen-reader runs of
  acceptance 13 (A6) need a person and are listed under *Open items needing a human decision*.
- **Why:** every M6 scope item (M6.1–M6.11) and acceptance criterion except A6 is done and
  checked; M7 (language foundation) is next.
- **Numbers:** docs only.
- **Open:** A6 with VoiceOver, Narrator, NVDA and Orca
  ([task](accessibility/m6-editing.md#a6-task-for-each-screen-reader)).

### M6 renames rewrite KPI and detail-rows DAX (M6.7, GT-6 s25)

- **What:** `pbi-session`'s rename plan rewrites, besides a measure's expression and format
  string, its `detailRowsDefinition` and the target, status and trend expressions of its `kpi`
  (TMDL objects, VertiPaq metadata and TMSL JSON, line arrays included), and a table's
  `defaultDetailRowsDefinition`; `dryRun` lists them (`kpi.statusExpression`, …), and
  `model.dependencies` and deletes list a measure whose KPI or detail rows use the object (a
  measure's own KPI is not its dependent). `pbi-tmdl`'s writer emits them in Desktop's form: after
  the scalars, each after a blank line, in the order KPI, detail rows, dynamic format string, then
  annotations; KPI properties in TOM's order with a multi-line expression one level below its
  property; a table's default detail rows after its scalars. TMSL `{"expression": …}` and KPI
  objects are converted to that form instead of being written as JSON text.
- **Why:** M6 open item "KPI and detail-rows expressions" (DESKTOP-PLAN M6 progress): a rename left
  them on the old name, which Desktop reports as an error in the KPI or the detail rows.
- **Numbers:** new tests `renames_rewrite_kpi_and_detail_rows_expressions` (pbi-session),
  `renames_rewrite_kpi_and_detail_rows_and_save_them` (pbi-ops save, fails without the fix; undo
  saves the original bytes) and `kpi_and_detail_rows_are_written_in_desktop_form` (pbi-tmdl:
  Desktop's text round-trips byte for byte, a scrambled file and TMSL JSON are written in it).
  GT-6 s25 in Power BI Desktop 2.159: a Desktop variant of the example with a table default detail
  rows, a measure's detail rows and a KPI measure (multi-line status expression), renamed by
  DashCraft (`Total Sales` → `GT6 Revenue`, `Orders[Line Amount]` → `GT6 Amount`) and saved, opens
  without errors: every measure, KPI and detail rows in state 1 (`INFO.KPIS`,
  `INFO.DETAILROWSDEFINITIONS`), the revenue 894300.5 as in `expected.json`, and
  `DETAILROWS` returns the 3,900 order lines through the measure's definition and the table
  default. Desktop's own save of that model equals DashCraft's `Orders.tmdl` byte for byte apart
  from line ends and the blank last line the source file did not have.
- **Open:** report-level measures of PBIR-Legacy and PBIX layouts (not read yet); KPI settings
  editors (M10.4).
- **Commit:** `7143b52` (PR #46).

### M6 canvas selections follow renames

- **What:** `pbi_model::renames_between(old, new)` lists the table, column and measure renames
  between two models, matched by `ObjectId` and ordered through placeholders so swaps and a table
  renamed with its columns come out right. `pbi-ops` canvas pages remember the model their view
  state names (`PageEntry::names`) and rewrite selected points and slicer conditions, saved and
  current, when the page is next used under a newer model (`follow_renames`); a click on a render
  made before the rename maps its point to the current names. `Target::update` reads the page and
  the model from one revision.
- **Why:** M6 open item "canvas state remapping after renames" (DESKTOP-PLAN M6 progress): a
  rename silently dropped a cross-filter ("cross-filter not applied") and `canvas.reset` restored
  stale names.
- **Numbers:** new tests `canvas_selections_follow_renames` (column and table rename, undo, redo,
  click on a stale render), `saved_and_current_selections_follow_model_renames` and
  `renames_between_models_follow_ids_through_swaps`; the integration test fails without the fix,
  and with either the click remap or `follow_renames` turned off. View state is not saved, so
  there is no Desktop ground-truth comparison.
- **Commit:** `4bd8669` (PR #45).

### M6 saves in Power BI Desktop's TMDL form (GT-4 s24, acceptance 14)

- **What:** `pbi-tmdl`'s writer emits Desktop's property order (columns, measures, relationships;
  a blank line before a measure's or calculation item's `formatStringDefinition`), and the
  source-preserving save keeps each edited file's line ends and blank last line (`FileStyle`; a
  new file takes the project's prevailing style, a conversion stays LF). `pbi-session` numbers
  calculation items without an `ordinal` by position, as Desktop does (was -1).
  `model.column.create` sets `isDataTypeInferred` when it infers the type. New fixtures
  `reference/groundtruth/gt4/s24/` (Desktop-saved `before/` and `after/`, `edits.json`) and the
  deviation list `reference/groundtruth/gt4/deviations.json`; test
  `crates/pbi-ops/tests/save.rs::edits_save_like_power_bi_desktop`. `gen_acme.py` writes the
  Desktop order and the Acme example is regenerated (five files).
- **Why:** M6 acceptance 14 (new-object TMDL matches Desktop's under the output contract,
  ADR-0014), the GAPS TMDL-writer row ("compare the writer with Desktop-saved TMDL").
- **Numbers:** after four measures, a calculated column and table, a hierarchy and a relationship,
  DashCraft writes the same 5 files as Desktop 2.159, equal byte for byte apart from new GUIDs and
  2 deviations (inferred data types). GT-6 s24: Desktop opens DashCraft's save with no dialog;
  after a refresh its values for the new objects equal DashCraft's (6 totals, 5 regions, 9
  channel-region shares) and `'Time Calc'` ordinals are 0–3 in both. Mutation checks: an empty
  deviation list and the old -1 ordinal each fail the test. `check_acme.py` 445/445; ground truth
  12/12 suites unchanged; perf P09, P10, P11 45.6, 55.2, 187 ms (ok); lint line numbers in 4
  tests and one CLI snapshot moved by one line.
- **Open:** Desktop's form of constructs no edit creates yet (roles, partitions, calculation
  items, perspectives; M26.1); TMDL with explicit ordinals out of document order still uses the
  explicit values (Desktop ignores them; Desktop never writes such files).
- **Commit:** `53bcb1c` (PR #44).

### M6 app editing (M6.8)

- **What:** the desktop app edits through the registry. The formula bar edits `Name = expression`
  of measures, calculated columns and calculated tables (rename + expression as one
  `doc.applyEdits` step); the Properties pane edits description, display folder and format string
  (`PropertyField.svelte`); Quick Access Toolbar Save, Undo and Redo name the step from
  `doc.history`; Ctrl/⌘+S, Ctrl/⌘+Shift+S, Ctrl/⌘+Z, Ctrl/⌘+Y; a `*` marker in the title; File →
  Save and Save as; an *Unsaved changes* question on close, window close and open
  (`lib/editing.ts`, `App.svelte`, `app/src-tauri`). `doc.applyEdits` gains an optional `label`
  (≤ 200 characters) and a `warnings` result, so a measure kept with DAX that does not parse is
  reported (a warning toast) instead of silently accepted. New scripts in `scripts/app-checks/`
  drive the built app over the WebView2 DevTools protocol (smoke and A1–A5 accessibility, with the
  IBM Equal Access checker and the UI Automation tree).
- **Why:** M6.8 (DESKTOP-PLAN), the GAPS §10 Save, Undo and Modeling rows.
- **Numbers:** 19 Node tests (`app/tests/editing.test.ts`), app test
  `edits_save_and_close_through_the_registry`, e2e assertions for the label and the warnings;
  smoke 23/23, accessibility 17/17 (A1–A5 pass; lowest contrast 5.98:1 text, 6.19:1 icons and
  input borders; 2 px focus rings), three contrast and focus failures fixed
  ([accessibility/m6-editing.md](accessibility/m6-editing.md)). GAPS §10: 2.0.
- **Open:** A6 (screen readers) not run; GT-4 (a Desktop save comparison of an app edit) not run;
  macOS Cmd+Q is not intercepted (Tauri); Ctrl+S saves without an uncommitted field draft.

### M6 visual groups move and resize (M6.5)

- **What:** `pbi-model` gains `Page::page_rect`, `parent_group` and `is_group_relative`
  (`PageRect`; groups nested up to 32 levels) and `placement_of`, which places a visual and, for a
  group, its members: children stored relative to the group keep their offsets on a move and scale
  on a resize in scale mode; children stored in page units are moved or scaled with it. The new
  `MoveVisual` command is evaluated when applied, so a move and a resize of the same visual in one
  `doc.applyEdits` compose; its inverse restores every old position. `pbi-render` resolves
  positions through `page_rect`. `report.visual.move`/`resize` and the `setVisualPosition` edit
  use it; results gain `members`. Tests: four group tests in `crates/pbi-model/src/report.rs`,
  `move_visual_takes_a_groups_members_along_and_undoes_exactly` and `batched_moves_compose`
  (`command/tests.rs`), `visual_groups_move_and_resize_with_their_members`
  (`crates/pbi-ops/tests/save.rs`: move, resize, batch, save of three files, reopen, render, undo).
- **Why:** the last open part of M6.5 (moving visual groups; DESKTOP-PLAN).
- **Numbers:** GT-6 s23: Desktop and DashCraft save the same positions for a group resize
  (1232 → 616 wide) and move (x 24 → 100) in 5 of 5 files; Desktop shows DashCraft's output at the
  expected page rectangles within 1 px. Render snapshots unchanged. M6.5 ticked.
- **Open:** a child stored in page units is told apart from a relative one by whether it lies
  inside the group, as the renderer did before; data column delete and hierarchy level edits come
  with the editors (M10).

## 2026-11-06

### M6 queries capture one revision, also with "View as" (M6.2, acceptance 8)

- **What:** `pbi-session` gains `Session::revision_snapshot` (`RevisionSnapshot`: revision,
  document, session model and query snapshot under one lock). Edits, undo and redo build the
  "View as" engine for the edited engine and model before publishing and install it in the same
  write lock as the model and revision. `pbi-render`'s `QueryEnv::from_revision`; the canvas
  (`canvas.*`) and `render.page` take the report, the page and the data from one capture, and
  canvas results gain `documentRevision`. Tests: `queries_return_the_revision_they_started_at`
  (queries polled once to capture their revision, edits committed while they are pending; values
  from the edited column and measure prove which revision was read; with and without "View as")
  and `concurrent_queries_never_mix_revisions` (three reader tasks against 60 seeded edit, undo
  and redo steps: no value, document, model or engine snapshot of two revisions) in
  `crates/pbi-session/tests/robustness.rs`; `canvas_renders_carry_their_revision_under_view_as`
  in `crates/pbi-ops/tests/security.rs`.
- **Why:** M6 acceptance 8 and the last open part of M6.2 (DESKTOP-PLAN).
- **Numbers:** the threaded test failed on every run before the fix (East view, revision 1:
  engine rows of revision 0); it passes on every run after it. The new session tests take about
  10 s in debug. M6.2 ticked.
- **Open:** derived-column changes (not revisioned) still rebuild the "View as" engine after
  publishing.

### M6 performance scenarios P09–P11 (acceptance 12)

- **What:** `crates/pbi-ops/examples/perf_ops.rs` runs three new `perf/budgets.toml` scenarios
  through `Registry::execute`, the path the app and agents take: P09 `edit-measure-requery`
  (`model.measure.update` of Acme's `Total Sales`, alternating text so every step commits, then
  `canvas.render` of the Executive Summary page), P10 `save-pbip-acme` (one edit, then `doc.save`
  into a copy of the project under `target/tmp/`) and P11 `refresh-acme-end-to-end` (`doc.open`,
  `data.refresh`, `canvas.render` of the first page, one process per sample).
- **Why:** M6 acceptance 12 (DESKTOP-PLAN); GAPS §8 had no end-to-end refresh scenario.
- **Numbers:** full `cargo xtask perf` on a 12-core x86-64 Windows desktop (no baseline entry, so
  no regression check there): P09 43.6 ms (target 50; edit 13 ms, re-render 32 ms), P10 55 ms
  (target 100; one file written), P11 184 ms (target 1000; open 42 ms, refresh of 11 tables
  91 ms, first page 30 ms). No code outside the new example changed, so P01–P08 cannot regress;
  on this machine they are all within target except P05 (365 ms against 250; 149 ms on the
  baseline machine). The CI runner's quick run: P09 53 ms (over its 50 ms target, within the
  150 ms maximum), P10 33 ms, P11 209 ms. GAPS §8 4.9 → 4.7 (three new rows, the end-to-end
  refresh gap closed).
- **Open:** record P09–P11 on the baseline machine; app open (P22) and webview canvas latency
  (P21) remain.

### M6 report-level measures follow renames (M6.7, part 3)

- **What:** `pbi-pbir` types `reportExtensions.json` (entities and measures, unknown keys kept);
  the report AST gains `Report::measures` (`ReportMeasure`, `MeasureReference`). Renames move the
  measures of a renamed table, rewrite their DAX at token level and their `references`;
  `model.dependencies` and deletes list them (`report-level measure 'T'[Name] of report 'R'`);
  a rename onto one of their names is refused (also in the model command). `doc.save` patches only
  the changed strings of `reportExtensions.json`, and the linter reads them from the AST. A report
  field whose name contains parentheses (`Orders.Sales (Scaled)`) was treated as a function call,
  so a table rename left it behind (LINT003); only a name before `(` makes a call now.
- **Why:** M6.7 (DESKTOP-PLAN), now ticked. GAPS `reportExtensions.json` row (1 → 2, File formats
  3.1 → 3.2) and "Editing commands" row.
- **Numbers:** new tests `report_extensions_are_report_measures` (pbir round-trip),
  `renames_follow_report_level_measures_whose_names_are_taken` (model),
  `renames_rewrite_report_level_measures` and `report_level_measures_are_dependents` (session),
  `renames_follow_into_report_level_measures` (core save), `renames_rewrite_report_level_measures_and_save_them`
  (ops save), linter and queryRef cases.

  GT-6 s22 (Desktop 2.159): three report-level measures on a card of the Desktop copy of Acme's
  Stores report, then DashCraft renamed a measure, a column and the table they use and saved.
  Desktop loaded the renamed report without an error; its own card query (read from
  `DISCOVER_SESSIONS` during a refresh) used the rewritten DAX and equals the one before the
  renames once names are mapped; the card values with the page filters are equal before and
  after, and so are the four unfiltered values. Negative control: a `dataType` outside the
  schema (`Int64`) makes Desktop refuse the report, so it validates the file at load.
- **Open:** report-level measures are not evaluated (M9.9) and not read from PBIR-Legacy or PBIX
  layouts (`modelExtensions`); a report-level measure cannot itself be renamed yet.

### M6 M-aware rename (M6.7, part 2)

- **What:** `model.object.rename` takes `expression` and renames an M query, parameter or
  function. `pbi-model` gains `RenameExpression`: Entity partitions that read the expression and
  the `PBI_QueryOrder` annotation follow (new `Snapshot::ModelAnnotations`), and query names are
  unique among expressions and M tables, ignoring case. A table rename does the same when the
  table is an M query. `pbi-m` gains `refs` (`free_names`, `rename_query`): a scope-aware walk
  rewrites free identifiers and `Section!member` names by span, quoting as `#"…"` when needed;
  the M lint now uses it. `pbi-session` rewrites the M of table partitions and shared
  expressions in the same undoable edit and reports `mRewrites`. A name the M cannot follow is
  refused: a step, parameter or field in scope where the old name is used, or a library function.
  M that does not parse is left as it is, with a warning.
- **Why:** M6.7 (DESKTOP-PLAN). GAPS "Editing commands" row ("M-aware rename").
- **Numbers:** new tests:
  - `e2e` `query_renames_rewrite_m_and_keep_the_expected_values`;
  - `save` `renamed_queries_save_and_reopen` (exact TMDL diff, reopen and refresh, undo restores
    the bytes);
  - session units `query_renames_rewrite_m_references` and
    `query_renames_the_m_cannot_follow_are_refused`;
  - model units `rename_expression_updates_the_query_order_and_undoes_exactly` and
    `queries_share_one_namespace`;
  - three `pbi-m` `refs` units;
  - expression renames and M-hostile names in the property test.

  GT-6 s21 (Desktop 2.159): DashCraft renamed five queries and parameters of the Desktop copy of
  Acme (15 references in seven tables and one expression). Desktop opened and refreshed the saved
  project, `INFO.EXPRESSIONS` and `INFO.PARTITIONS` show the new names and the rewritten M, and
  all 643 recorded cases equal the committed recordings. The same renames on the example refresh
  in DashCraft to the expected values.
- **Open:** report-level measures (`reportExtensions.json`) are not renamed or rewritten, so
  M6.7 stays unticked. Descriptions that mention the old name are left unchanged.

### M6 calculated table updates (M6.4, part 2)

- **What:** `model.table.update` and the `updateTable` edit operation (`pbi-ops`): a calculated
  table's expression, or any table's description and visibility. The new expression's columns are
  inferred over the loaded data (`Session::infer_changed_table_columns`) and merged at apply time
  (`UpdateTable`, so earlier edits in a `doc.applyEdits` group are kept): columns with the same
  source column, else name, keep their ids and properties; new ones are added like a created
  table's; the rest are removed through `pbi-model`'s new `SetTableDefinition` (exact inverse).
  Self-references are circular now: `pbi_dax::refers_to_table` (new) and the refresh plan keeps a
  column's or table's edge to itself.
- **Why:** completes M6.4 (DESKTOP-PLAN); GAPS "Editing commands" row ("calculated table expression
  edits").
- **Numbers:** registry 72 → 73 commands. New tests: `e2e`
  `calculated_table_update_through_the_registry`, `save` `updated_table_saves_and_reopens`, model
  unit `table_definitions_change_columns_with_exact_inverses`, plan unit
  `objects_naming_themselves_are_circular`, `refers_to_table` asserts in `pbi-dax` `eval`, a
  self-referencing column refusal and an `updateTable` property case. GT-6 s20 (Desktop 2.159):
  two column updates and one table update; values recomputed in the session equal Desktop's after
  its refresh; 38 recorded cases changed and all 643 match.
- **Open:** changes to calculation items are not tracked; page queries still run again after any
  model edit.
- **Commit:** `5193c24` (PR #37).

### M6 recalculation (M6.4, part 1)

- **What:** `pbi-session::recalc` (new): after every edit, undo and redo the session compares the
  old and new model and recomputes, in the same publication and in dependency order, the
  calculated tables and columns the change reaches (created or changed expressions and types,
  users of a measure whose expression changed, readers of other tables when relationships
  change, references that resolve differently after a create or delete, then everything
  downstream and the derived columns). A failure of the edited object refuses the edit; any other
  failure drops the stale values (`Engine::remove_column`, `Engine::unload_table`, new) and
  reports the error. `pbi-model`: `SetColumnProperties`. `pbi-ops`: `model.column.update` and the
  `updateColumn` edit operation; edit outputs carry `recalculated`. `created.rs` lost its own
  compute path to `recalc.rs`.
- **Why:** M6.4 and DESKTOP-PLAN acceptance criterion 3 (a measure edit is visible without a
  refresh); GAPS "Model editing" row.
- **Numbers:** registry 71 → 72 commands. 4 new tests (`e2e`
  `edits_recalculate_dependants_without_a_refresh`, `save` `updated_columns_save_and_reopen`,
  engine `stale_columns_and_tables_can_be_dropped`, model unit
  `set_column_properties_patches_calculated_columns_only`) and a `model.column.update` case in
  the property generator. On Acme a relationship change recomputes 7 calculated objects; a
  measure edit recomputes only the columns that use it, and `measures.values` after it emits no M
  progress event.
- **Commit:** `3b6a017` (PR #36).
- **Open:** `model.table.update` (calculated table expression edits) completes M6.4; changes to
  calculation items are not tracked; page queries still run again after any model edit.

### M6 CLI as registry wrappers (M6.10)

- **What:** `dashcraft detect`, `inspect` and `lint` run `doc.detect`, `doc.open` + `doc.inspect`
  and `doc.lint` through a small registry (`crates/dashcraft-cli/src/ops.rs`, current-thread
  runtime) and only print the result; the text writers deserialize the command outputs.
  `dashcraft run --save` saves the `--doc` project after a successful command or batch.
  `pbi-ops`: `open_error` names a missing input once, `doc.lint` findings carry `object` (the
  object id), the output types derive `Deserialize`. `pbi-core`: the PBIP fingerprint scan
  checks its caps from the directory listing before reading and stamps files on up to 8 threads.
  Paths that are not valid Unicode are now refused by these subcommands (parameters are JSON).
- **Why:** M6.10, the ADR-0007 migration plan; GAPS §9 "CLI over the registry" 3.2 → 3.4.
- **Numbers:** outputs and exit codes byte-identical before and after on 374 runs (17 inputs,
  from Acme in five shapes to broken, junk and missing files, × 22 flag sets). Release, median
  of 21: Acme `inspect` 45 → 58 ms, `lint` 45 → 58 ms, `detect` 13 → 15 ms; Regional Sales
  `inspect` 63 → 78 ms. The extra time is `doc.open` itself (session document clone 1–5 ms,
  project fingerprint, runtime); the scan went 8.8 → 5.8 ms on Acme, which also speeds up the
  save conflict check. 4 new tests (2 `e2e`, 1 `error` unit, 1 CLI `run --save`).
- **Open:** `refresh`, `measures`, `export`, `render` and `convert` as wrappers; sharing the
  opened document (`Arc`) to drop the clone in `Session::from_opened`.
### M6 registry fuzzing (M6.11)

- **What:** `crates/pbi-ops/tests/property.rs`: every catalog command called through
  `Registry::execute` with its examples, with parameters derived from its JSON Schema and with
  40 random mutations each (no `internal` error, no hang); seeded random edit, undo and redo
  sequences on Acme (refused and no-op steps change nothing, `undo` back to a document equal to
  the original) and on loaded data (save as PBIP, reopen with the same shape, undo all, same
  measure values). `DASHCRAFT_PROPERTY_STEPS` / `DASHCRAFT_PROPERTY_SEED` set length and seed;
  a nightly `property` job in `fuzz.yml` runs 10,000 steps with the run number as seed
  ([docs/ci.md](ci.md#registry-property-tests)). Fix found by it: `pbi-core` save refused a
  report whose field was renamed and then deleted (`net_renames` only followed surviving
  objects); `deleted_renames` now infers those renames from the report fields, chains and swaps
  included, and `check_reports` still verifies the result. GAPS §9 2.9 → 3.2, §12 3.1 → 3.3.
- **Why:** M6.11 and M6 acceptance 10; GAPS §9 and §12 "parameter fuzzing".
- **Numbers:** parameter test 8,688 calls over 71 commands; random edits 10,000 steps (5,118
  committed, 182 no-ops, 4,700 refused) in 85.7 s release, 231.5 s debug; loaded data 1,000
  steps per seed, seeds 3 to 7 all green in 10–14 s release (debug default 250 steps, about 9 s).
  2 new save tests in `pbi-core` (unit and integration).
### M6 agent editing over MCP (M6.9)

- **What:** `crates/pbi-ops/tests/mcp.rs::agents_edit_and_save_only_inside_write_roots`: an MCP
  server limited to `--allow edit --allow workspace` lists the edit, undo and save tools (not
  `render_page`); an agent edits a project under a read root and one under a write root, saving
  the first fails with `pathDenied` (bytes unchanged, still dirty) and the second writes only
  `tables/Sales.tmdl`. `docs/MCP.md` gains an Editing section (the `edit` tools) and the
  `doc_saveAs` write-root rule; DESKTOP-PLAN M6.9 ticked.
- **Why:** M6.9 and M6 acceptance 11; the commands were already agent-safe in the `edit` category
  (ADR-0012), the end-to-end confinement through MCP was untested.
- **Numbers:** 1 new test; `pbi-ops` MCP tests 10 → 11. No code change.
### M6 calculated table create (M6.5)

- **What:** `pbi-model` gets `CreateTable` (calculated tables only: one calculated partition, no
  measures or hierarchies, a free name, distinct valid column names of the calculated kinds; fresh
  ids for the table, its columns and partition; the inverse is `DeleteTable`, so redo keeps the
  ids). `pbi-session` infers the columns from the expression's result
  (`Session::infer_table_columns`: types from the values, else from the source column) and computes
  a created table in the edit's publication when the data is loaded (`compute_created_tables`,
  recorded as a calculated refresh; restored tables keep their data and record). `pbi-ops` adds
  `model.table.create` and the `createTable` edit: random `lineageTag`s, `isNameInferred`,
  `isDataTypeInferred`, `sourceColumn` from the result as Desktop names it (`Stores[Region]`,
  `Web Sessions[Device]` unquoted, `[Pages]`), sum for
  numeric columns, a calculated partition named after the table.
- **Why:** M6.5 core editing commands: calculated table create (a "Still open" item).
- **Numbers:** 3 new tests (`pbi-model`
  `calculated_tables_are_created_with_fresh_ids_and_deleted_by_undo`;
  `e2e.rs::calculated_table_create_through_the_registry`: typed from the source columns before a
  refresh and computed by it, an `ADDCOLUMNS` table over `'Web Sessions'` typed from its values and
  computed at once, a calculated column on a created table, eight refusals, undo/redo, a group;
  `save.rs::created_table_saves_and_reopens`: the exact new table file, `ref table` appended to
  `model.tmdl`, reopen and refresh, undo saves the original project). Registry 70 → 71 commands.
  GT-6 with Desktop 2.159 (s19: `Regions`, `'Device Pages'`, `'Store Places'`): the first run
  wrote `'Web Sessions'[Device]` quoted and Desktop replaced that column (the one metadata
  difference, caught by the ground-truth test); with Desktop's unquoted form it keeps every column
  as written, its rows equal DashCraft's, and DashCraft matches all 643 cases on the edited model.

### M6 hierarchy create and delete (M6.5)

- **What:** `pbi-model` gets `CreateHierarchy` and `DeleteHierarchy` (exact inverses that keep
  the hierarchy's and levels' ids; refuses names taken by a column, measure or hierarchy of the
  table, no levels, duplicate level names, missing or row-number columns and a column used
  twice). `pbi-session::object_dependents` lists the report filters and visual fields on a
  hierarchy's levels. `pbi-ops` adds `model.hierarchy.create` and `model.hierarchy.delete` and
  the `createHierarchy` and `deleteHierarchy` edits: random `lineageTag`s on the hierarchy and
  each level, saved in place with levels in order and no ordinals, as Desktop writes them.
- **Why:** M6.5 core editing commands: hierarchy create and delete (a "Still open" item; no data
  recomputation needed, hierarchies do not change the engine).
- **Numbers:** 4 new tests (`pbi-model`
  `hierarchies_are_created_and_deleted_with_exact_inverses`; `pbi-session`
  `hierarchy_dependents_are_the_report_fields_on_its_levels`: the Product Detail matrix on
  `Products[Product Hierarchy]`; `e2e.rs::hierarchy_create_and_delete_through_the_registry`;
  `save.rs::created_and_deleted_hierarchies_save_and_reopen`: only `Stores.tmdl` written, the
  exact block where Geography was, reopen). Registry 68 → 70 commands. GT-6 with Desktop 2.159
  (s18: Geography deleted, three hierarchies created, one on a table without any and one with a
  calculated-column level): Desktop opens and refreshes the saved project and lists the
  hierarchies with their levels in order on the right columns; its 643 answers equal the
  committed recordings and DashCraft matches all of them.
### M6 calculated column create (M6.5)

- **What:** `pbi-model` gets `CreateColumn` (calculated columns only; refuses calculation groups,
  invalid names and names taken by a column or measure of the table; inverse `DeleteColumn`).
  `pbi-session` computes a created column at once over the loaded data in the same publication
  as the edit (`created.rs`; redo re-attaches the kept data instead, a table without rows gets
  it at the next refresh) and infers a new column's data type from its values
  (`Session::infer_column_type`). `pbi-ops` adds `model.column.create` and the `createColumn`
  edit: data type inferred unless `dataType` is given, Desktop's defaults (random `lineageTag`,
  sum for numeric columns, `SummarizationSetBy` annotation).
- **Why:** M6.5 core editing commands: column create (the next "Still open" item after deletes).
- **Numbers:** 3 new tests (`pbi-model` `create_column_round_trips_and_checks`;
  `e2e.rs::calculated_column_create_through_the_registry`: typed create before a refresh and the
  refresh computes it, inferred `string` and `int64`, 8 refusals, a blank column with a given
  type, undo/redo, a group of two dependent columns and a measure, refresh after the group;
  `save.rs::created_column_saves_and_reopens`: only `Orders.tmdl` written, the exact block
  before the partition, reopen, undo saves the original project; a document opened with
  `autoRefresh` loads its data before inferring). Registry 67 → 68 commands. GT-6 with Desktop
  2.159 (s17: three created columns, types inferred on the example, saved on a Desktop copy):
  Desktop refreshes the project without an error and lists the same types and summarization,
  and DashCraft's results on the edited example match Desktop's on all 643 recorded cases,
  including every row of the new columns in the `Orders` and `Customers` tables.
- **Still open:** calculated table create, data column delete with M editing, hierarchy create
  and delete; a created column is refused when its DAX fails on a row (Desktop keeps it with an
  error).
### M6 calculated column and table delete (M6.5)

- **What:** `pbi-model` gets `DeleteColumn` (calculated columns only; refuses data, calculated
  table and row-number columns and columns used by a relationship, hierarchy level or sort-by
  column) and `DeleteTable` (cascades the table's relationships and the roles' table permissions
  on it), both with exact inverses that restore everything in place with the same ids.
  `pbi-engine` can detach a stored table or column and attach it again with its data and cached
  encodings (`Storage::detach`/`attach`, `Engine::sync_model_with_edits`). `pbi-session` takes a
  deleted object's data out of the engine and keeps it by object id (`removed.rs`), so the object
  can no longer be queried and undo restores its data without a refresh; refresh results follow;
  `delete_dependents` leaves out the DAX of a deleted table itself. `pbi-ops` adds
  `model.column.delete`, `model.table.delete` and the `deleteColumn`/`deleteTable` edits
  (`dryRun`, dependents, `removed`, warnings).
- **Why:** M6.5 core editing commands: column and table delete. Before the fix in the session,
  a deleted column stayed queryable because the engine kept its stored data.
- **Numbers:** 8 new tests (3 `pbi-model` commands; `pbi-engine` detach/attach; `pbi-session`
  delete dependents and `robustness.rs::deletes_take_the_data_out_and_undo_puts_it_back`;
  `e2e.rs::column_and_table_delete_through_the_registry`: refusals, a deleted column fails in
  DAX and undo gives the same values, a column with its two measures in one group, Stores with
  its 2 relationships and 2 role filters; `save.rs::column_and_table_deletes_save_and_reopen`:
  5 files written, `Stores.tmdl` deleted, reopen without them, undo saves the original project
  byte for byte). Registry 65 → 67 commands. GT-6 with Desktop 2.159 (s16, the same deletes
  saved on a Desktop copy and on the example): Desktop opens the project with 10 tables, and
  DashCraft's results on the edited example match Desktop's on all 643 recorded cases, including
  the 226 measure and 12 RLS cases that now fail in both because Stores is gone.
- **Still open:** table and column create (type inference, recalculation of the new object only),
  data column delete with M editing, hierarchy create and delete; Desktop's own refusals of a
  delete (it may allow deleting a column a relationship uses) not checked.

### M6 relationship update (M6.5) and GT-6 relationship checks

- **What:** `pbi-model` gets `SetRelationshipProperties` (cardinalities, cross-filter and security
  filter directions, active flag; patches only the given properties, exact unchecked inverse) and
  shares the "one active path between two tables" check with creation. `pbi-ops` adds
  `model.relationship.update` and the `updateRelationship` edit; relationship edit results gain
  the relationship's properties. No session change: any relationship change already rebuilds the
  engine's relationship graph.
- **Why:** M6.5 core editing commands, the last relationship part; GT-6 had not yet checked a
  saved relationship edit in Desktop.
- **Numbers:** 3 new tests (`pbi-model` round trip and refusals;
  `e2e.rs::relationship_update_through_the_registry`: Customers one-direction changes a
  cross-filtered count, activation refused alone and accepted as a swap in one group, which gives
  `Shipped Sales` through the active Ship Date, 4 refusals, undo restores everything;
  `save.rs::relationship_updates_save_and_reopen`: only `relationships.tmdl`, only the changed
  properties written, reopen equal, undo saves the original bytes). Registry 64 → 65 commands.
  GT-6 with Desktop 2.159 ([reference/groundtruth](../reference/groundtruth/README.md#writer-checks-gt-6-desktop-opens-dashcrafts-output)):
  delete and recreate (s13) identical on 642 of 643 recorded cases, the last one the same
  relationship rows in another order; four updates (s14) change 196 of 601 measure cases in Desktop, and
  DashCraft's results on the edited example match Desktop's on all 643 cases; security filtering
  in both directions with a one-direction cross filter (s15) is refused by Desktop with the rule
  DashCraft applies.
- **Still open:** table, column and hierarchy create and delete; a committed edited-model ground
  truth suite (s14 ran from a scratch reference root).

### M6 relationship create and delete (M6.5)

- **What:** `pbi-model` gets `CreateRelationship` and `DeleteRelationship` (exact inverses; a
  deleted relationship is restored in place with its id; checks for names, columns, one table,
  duplicate column pairs and a second active path between two tables). `pbi-session` finds a
  relationship's dependents (DAX naming both ends in `USERELATIONSHIP`/`CROSSFILTER`; unparsable
  DAX mentioning both is a warning) and names new relationships with the identifier generator.
  `pbi-ops` adds `model.relationship.create`/`delete`, the `createRelationship`/
  `deleteRelationship` edits, relationship ids in `model.relationships` and
  `model.dependencies`. `pbi-tmdl::write_model_preserving` keeps the text of every unchanged
  top-level object in an edited file (relationships, expressions, …), checked by a reparse.
- **Why:** M6.5 core editing commands, the relationship part; the save test showed that the
  first edit to `relationships.tmdl` rewrote every other relationship canonically (Acme's
  spelled-out `toCardinality: one` was dropped).
- **Numbers:** 6 new tests (3 `pbi-model` command tests, 1 `pbi-session` dependents test,
  `e2e.rs::relationship_create_and_delete_through_the_registry`: Store filtering off and on
  through delete, undo, recreate and a grouped edit, the Ship Date dependent, 9 refusals, a
  failing group and the `ENG002` warning; `save.rs::relationship_edits_save_and_reopen`: only
  `relationships.tmdl` written, GUID name, reopen, undo saves the original bytes; plus a
  `pbi-tmdl` writer unit test). Registry 62 → 64 commands.
- **Still open:** relationship update (cardinality, direction, activation), table, column and
  hierarchy create and delete; a GT-6 Desktop check of a saved relationship edit.

### M6 `doc/changed` notifications (M6.3, ADR-0012 §5)

- **What:** `pbi-ops` gets `DocChangedEvent` (document, command, revision, dirty flag,
  `ObjectChange`s of kind added, changed, renamed or removed, the new path after Save As) and a
  per-request sink on `Ctx` (`with_doc_changed`), inherited by `commands.batch` steps. Every
  committed edit command, `doc.undo`/`doc.redo`, a save that clears the dirty flag and
  `doc.saveAs` publish one; no-op, failed and dry-run edits publish none. `serve` sends them as
  `doc/changed` notifications through the response outbox before the response (also for
  requests without an id); `mcp` sends them as `notifications/message` at level `info` with
  `data.event: "doc/changed"` (MCP has no document-change notification, and custom methods
  break strict clients), subject to the client's log level.
- **Why:** the last open part of M6.3; editors, agents and the app (M6.8) need to know what an
  edit changed without polling `doc.history`.
- **Numbers:** 2 new tests (`rpc.rs::committed_edits_send_doc_changed_before_the_response`:
  create, no-op, failure, rename, a batch with a failing step, an undo notification, two saves;
  `mcp.rs::committed_edits_are_logged_as_doc_changed`); `pbi-ops` tests 78 passed, 0 failed.
  M6.3 is ticked.
- **Still open:** the app's `ops_execute` adapter does not forward the events yet (M6.8).

### M6 lineage tags for new objects (ADR-0014 §5)

- **What:** `pbi-session` gets an injectable `IdentifierGenerator` (`SessionOptions::identifiers`):
  `RandomIdentifiers` (version 4 GUIDs through `uuid`, already in the tree through DataFusion;
  now a workspace dependency) by default, `SequentialIdentifiers` for tests.
  `Session::new_lineage_tag` returns a tag not in use in the model, or none when the model has no
  lineage tags at all (no format upgrade). `model.measure.create` and the `createMeasure` edit of
  `doc.applyEdits` put it in the new measure before the command is built, so undo, redo, saves and
  a reopen keep it.
- **Why:** M6.6 remainder and ADR-0014 §5: Desktop gives every object a `lineageTag`; DashCraft's
  new measures had none.
- **Numbers:** 4 new tests (`pbi-session` unit tests for both generators; `pbi-ops/tests/save.rs`
  `new_measures_get_lineage_tags_from_the_injected_generator` and
  `lineage_tags_are_random_and_only_where_the_model_uses_them`); `pbi-ops` and `pbi-session`
  tests: 165 passed, 0 failed. GT-6 with Desktop 2.159: Desktop reads both generated tags back
  unchanged (`INFO.MEASURES()`), `acme-measures` 601/601 and `acme-calcgroup-rls` 26/26 identical
  to the committed recordings, `acme-metadata` differs only by the two added measures.
- **Still open:** tags for tables, columns, hierarchies and relationships come with their create
  commands (M6.5).

### M6 Save As: `doc.saveAs` writes a new PBIP project from any source

- **What:**
  - `pbi-core::save_as_project` writes the current document (`edited_document`: the last saved
    state with the edits, renames checked against the reports) with the full PBIP writer into a
    new folder (`safe_path`, atomic writes, `PartialSave` on a failure part-way), reads it back
    and makes that the new save baseline with the session's document, so object ids, undo
    history and later in-place saves all carry on. TMSL shadow properties are stripped from a
    PBIT or `model.bim` source's model (otherwise a no-op save afterwards would rewrite files).
  - `pbi-session::Session::save_as` and `edited_opened`; `pbi-ops` adds `doc.saveAs` (write
    root, empty folder unless `overwrite`, warnings for data not carried over) and moves the
    workspace entry to the new `.pbip`. `convert.pbip` and `convert.pbit` write the edited
    document instead of refusing a dirty one.
- **Why:** M6.6 (ADR-0013): a PBIX, PBIT or `model.bim` could be edited but not saved.
- **Still open:** `lineageTag` for new objects (ADR-0014), Save As PBIT keeps going through
  `convert.pbit` (no re-association), object origins keep the old source locations.
- **Numbers:**
  - The catalog grows from 61 to 62 core commands.
  - New tests: `pbi-core` 3 (`save_as_writes_the_edited_project_and_later_saves_go_there`,
    `samples_save_as_pbip_with_an_equal_structure` on all 4 committed samples,
    `legacy_layout_report_changes_are_refused`), `pbi-ops` 3 (`save.rs`: the synthetic Mini project and
    the Revenue Opportunities PBIX; `e2e.rs::save_as_project_keeps_the_expected_values`),
    and Save As cases added to the model-folder, path-policy, conversion and `Send` tests.
  - Acceptance criteria 5 (structure of every sample after Save As) and the PBIP half of 6
    (Acme saved as, refreshed through the blob map: every `expected.json` value) pass.
  - GT-6 with Desktop 2.159: Acme as a PBIT, edited and saved as a PBIP, opens and refreshes in
    Desktop and records identically to the edited PBIP saved in place (643 of 643 cases;
    `reference/groundtruth/README.md`).

### M6 save: `doc.save` writes edits into the opened PBIP project

- **What:**
  - `pbi-core::save_in_place` compares the PBIP files written for the last saved state with those
    for the current document and writes only the files that differ, each through a temporary
    file and a rename. A `ProjectFingerprint` of every project file (taken at open and after each
    save) turns files changed on disk into `CoreError::Conflict`; `overwrite` saves anyway. A
    failure part-way is `CoreError::PartialSave` with the written and failed files.
  - Renames move a table's `.tmdl` file (case-only renames too) and are applied to the report
    JSON (`queryRef`, `nativeQueryRef`, filters, bookmarks) by editing the JSON values, not by
    re-serializing; visual moves patch only the `position` numbers. Report changes it cannot
    write (and any change to a legacy `report.json` report) refuse the save.
  - `pbi-session::Session::save` runs it off the async runtime and advances the saved revision
    only when every write succeeded. `pbi-ops` adds `doc.save` (`overwrite`, write-root check)
    and the `conflict` error code (JSON-RPC -32008, exit code 1); the app shows it as "Changed on
    disk".
  - Two fixes found by the save tests: the source-preserving TMDL writer dropped a single `ref
    table` line (so a one-table rename could not reach `model.tmdl`), and a column rename left
    the default aggregation label (`Sum of Amount`) behind, which LINT003 reported.
- **Why:** M6.6 (ADR-0013, ADR-0014): edits could not leave the process.
- **Still open:** `doc.saveAs` (PBIP from PBIX, PBIT and `model.bim`; conversions still refuse
  dirty documents), `lineageTag` generation for new objects, report-level measures
  (`reportExtensions.json`) in renames, the `PBI_QueryOrder` annotation after a table rename,
  non-English aggregation labels.
- **Numbers:**
  - The catalog grows from 60 to 61 core commands.
  - New tests: `pbi-core` 8 integration (`tests/save_in_place.rs`, on a copy of Acme Outfitters)
    and 2 unit, `pbi-ops` 3 end-to-end (`tests/save.rs`) plus a `pathDenied` case, one each in
    `pbi-tmdl` and `pbi-model`.
  - Acceptance criteria 1 (a no-op save of Acme writes nothing), 2 (a measure edit changes only
    its line in `Orders.tmdl`; undo and save restore the bytes) and the save half of 7 (edit,
    save, reopen in a second workspace, undo, save the original bytes) pass.
  - Desktop ground truth unchanged: 2,241 of 2,242 equal, 1 ledger entry.
  - Desktop writer check (GT-6, Desktop 2.159, `reference/groundtruth/README.md`): a batch that
    renames `Orders[Line Amount]`, updates and creates measures, sets a description and moves a
    visual, then saves, wrote 3 files; Desktop opened the result without a repair, and its
    recordings match the committed ones on all 601 measure and 26 calculation-group/RLS cases.
    The 5 differing metadata and table cases differ only by the edits (same 3,900 Orders rows).
- **Commits:** `a8d0411` (feature), plus the GT-6 notes.

### CI: macOS jobs on `macos-15`

- **What:** `ci.yml` (`rust`, `app`) and `release.yml` (`bundle`, CLI archives) run on `macos-15`
  instead of `macos-14`; `docs/ci.md` and `docs/packaging.md` follow.
- **Why:** GitHub retired the macOS 14 runner image on 2026-11-02. Since then the macOS jobs were
  cancelled before their first step (`The configuration 'macos-14-arm64-us-default' is not
  supported`), so CI on `main` reported "cancelled" from #19 onwards. Binaries keep rustc's
  default deployment targets (none is pinned), so the minimum macOS version does not change.

## 2026-11-05

### M6 measure create, update and delete, and object dependents

- **What:**
  - `pbi-model` gains `CreateMeasure`, `DeleteMeasure` (its inverse restores the measure in place
    with its id) and `SetMeasureProperties` (patches only the given fields). Measure names are
    checked by one shared `check_measure_name`. Undo never rewinds the id generator, so ids are
    never reused.
  - `pbi-session::object_dependents` lists what refers to a table, column or measure: DAX
    (through the rename planner in probe mode), relationships, hierarchy levels, sort-by
    columns, role permissions, and report, page and visual fields and filters.
    `dax_syntax_error` checks an expression.
  - `pbi-ops` adds `model.measure.create`, `model.measure.update` and `model.measure.delete`
    (`dryRun` lists the dependents only), plus the `doc.applyEdits` variants `createMeasure`,
    `updateMeasure` and `deleteMeasure`. DAX that does not parse is kept, with a warning.
  - `model.dependencies` (read-only, agent-safe) lists the dependents of any table, column or
    measure.
- **Why:** M6.5 progress: measure create and delete were the first create/delete gap in
  [GAPS](GAPS.md).
- **Still open:** table, column, hierarchy and relationship create and delete; `lineageTag` for
  new measures (assigned at save, ADR-0014); save; notifications; recalculation.
- **Numbers:**
  - The catalog grows from 56 to 60 core commands.
  - `cargo test -p pbi-ops`: 68 tests, including 10 end-to-end.
  - `pbi-model`: 63 library tests. `pbi-session`: 43 library tests.
  - Full workspace gate (branch `feat/m6-editing`): `cargo clippy --all-targets --locked -D
    warnings` clean; `cargo test --locked`: 1,289 passed, 0 failed, 2 ignored. `Session::document()`
    now returns an `Arc` snapshot, so pbi-render test helpers and the `perf_render` example hold
    the snapshot instead of borrowing from it.
  - Desktop ground truth unchanged: 2,242 recorded queries in 11 corpora, 2,241 equal, 1 in the
    ledger, 0 new. The edit commands themselves are not compared with Desktop yet (that needs save,
    M6.6).
  - The `docs/CLI.md` editing example (create, query, rename dry run, undo, history) runs on the
    Acme Outfitters example.

### M6 rename, description and visual position commands

- **What:**
  - `pbi-dax::rename` rewrites table, column and measure references at token level and keeps
    formatting and comments.
  - `pbi-engine` renames stored tables and columns without reloading them.
  - `pbi-model::History` gains checked edits (`CommandError::Rejected`).
  - `pbi-session::edit` (`RenameObject`, `preview_rename`) rewrites every DAX location of the
    model and its derived columns. It prepares the new model and engine, then commits both in one
    step.
  - Report-only edits (visual moves) keep the engine, the session model and the canvas cache.
  - `pbi-ops` adds four commands: `model.object.rename` (with `dryRun`),
    `model.object.setDescription`, `report.visual.move` and `report.visual.resize`. It also adds
    the `doc.applyEdits` variants `renameObject` and `setVisualPosition`.
- **Why:** M6.5 and M6.7 progress. Acceptance criterion 4 passes: renaming
  `Orders[Line Amount]` rewrites every DAX reference, and all values of `expected.json` still hold
  before and after a refresh.
- **Still open:**
  - M references are warned about, not rewritten.
  - KPI and detail-rows expressions and report-level measures are not rewritten.
  - Measure edits do not recompute dependent columns.
  - Visual groups cannot be moved.
  - There is no save or writer sync yet.
- **Numbers:**
  - The catalog grows from 52 to 56 core commands.
  - `cargo test -p pbi-ops`: 67 tests, including 9 end-to-end.
  - `pbi-session`: 41 library tests and 12 robustness tests.

### M6 session and registry edit slice

- **What:** `pbi-session` now publishes command, undo and redo changes into its opened document,
  engine model and session model while preserving active derived columns; `pbi-ops` exposes
  `model.setMeasureExpression`, atomic grouped `doc.applyEdits` and document history commands.
  Measure inspection returns stable object IDs, and the generated command catalog is current.
  DAX query, single-measure and all-measures outputs include the revision captured alongside
  their engine snapshot and model. Dirty documents require explicit discard to close, and
  conversion commands refuse unsaved edits while their writer sources remain unsynchronized.
- **Why:** Establishes the first M6.2/M6.3 runtime path and the first M6.5 editing command. The
  writer-source revision, remaining edit commands, save path, app UI and change notifications are
  still open.
- **Numbers:** `cargo test -p pbi-session` (79 tests), `cargo test -p pbi-ops` (65 tests);
  registry integration covers edit, no-op, one-step grouped undo/redo and query revision
  reporting. The catalog now has 52 core commands.

### M6 transactional history foundation

- **What:** `pbi-model::History` now applies commands against a staged document copy, so failed
  grouped edits cannot publish partial changes. Changed edits, undo and redo advance a monotonic
  revision; no-ops preserve redo and return `changed: false`; dirty state follows the current
  content identity. Added status-bearing operation results.
- **Why:** First runtime increment toward M6.2–M6.3, following the accepted revision and
  transaction contract in ADR-0012. Registry ownership, a unified session/writer revision and
  the rest of M6 remain open.
- **Numbers:** `cargo test -p pbi-model` (58 tests) and crate clippy pass.

### M6.1 architecture decisions

- **What:** Accepted ADR-0012 through ADR-0015 for revisioned document state, save conflicts and
  recovery, the serialization contract and identifiers, and accessibility verification.
  Corrected the Desktop plan's provisional ADR numbering so it no longer collides with accepted
  ADR-0010 (DashCraft naming) and ADR-0011 (Desktop ground truth).
- **Why:** M6 is the next Roadmap milestone and addresses the 0-scored GAPS rows for registry
  editing and app editing; M6.1 must settle the state, save and verification contracts before
  implementation.
- **Numbers:** Four decision records.

### Field-parameter composite keys

- **What:** `pbi-model` `Column::group_by_columns()` reads `relatedColumnDetails` (TMDL
  `groupByColumn`, TOM JSON `groupByColumns`); `pbi-vertipaq` fills it from the
  `RelatedColumnDetails`/`GroupByColumn` metadata tables; the TMDL and TMSL writers convert
  between the two shapes; `pbi-dax` raises Desktop's "part of composite key" error when such a
  column is aggregated alone (and the storage-engine fast path falls back to that error).
- **Why:** The last fixable deviation-ledger entry (`samples-corporate-spend`
  `column/toggle-for-*`).
- **Numbers:** `samples-corporate-spend` 79/79 equal to Desktop without a ledger entry; ledger
  2 → 1. The remaining entry (`format` `measure/sales-scaled`, by design) is the 15th digit of a
  double sum: adding Orders[Line Amount] in Desktop's own storage row order reproduces its
  …003 exactly, the CSV order gives …4999999986 and DashCraft's per-dictionary-value sum …001, so
  matching would mean reproducing VertiPaq's row-reordering heuristic.

### DAX context semantics from the ground-truth ledger

- **What:** `pbi-dax` gains per-iteration `ALLSELECTED` shadow contexts (`context::Shadow`,
  resolved lazily; `SUMMARIZECOLUMNS` rows and `EvalContext::with_allselected` set fixed ones), the
  table form of `ALLSELECTED` and the implied relation of the window functions keep only the
  shadow's filters on their scope, `KEEPFILTERS` on an iterator's table (context transition
  intersects), and Desktop's errors for `KEEPFILTERS`/`IGNORE` outside their valid positions and
  for a many-to-one-side-only `CROSSFILTER`. 30 new `functions.rs` cases, recorded in Desktop.
- **Why:** The 4 non-trivial entries of the deviation ledger (1 bug, 3 lenient) and the GAPS DAX
  row "`ALLSELECTED` shadow contexts".
- **Numbers:** `dax-functions` 522 → 552 cases, all equal to Desktop; ledger 6 → 2 entries (left:
  float summation order in one `FORMAT` case, by design; the field-parameter composite key of one
  MIT sample). GAPS DAX 3.8 → 3.9.

### Power BI Desktop ground truth (ADR-0011)

- **What:** A one-to-one oracle for computed values. `scripts/groundtruth/` records DAX query
  corpora (`gen_corpus.py`) against Power BI Desktop 2.159's local engine over ADOMD.NET
  (`Record-GroundTruth.ps1`, black-box); `reference/groundtruth/` commits corpora, recordings,
  the deviation ledger and the generated `dax-lab` fixture (`gen_dax_lab.py`, the models of
  `functions.rs`); `crates/pbi-session/tests/groundtruth.rs` compares value by value;
  `cargo xtask groundtruth [--check]` reports status. Writer check (GT-6): DashCraft's PBIP and
  PBIT opened in Desktop, re-recorded and compared (`compare_recordings.py`). New lint rule
  `LINT018`; M `List.Dates` and siblings.
- **Why:** Every test so far encoded our own reading of Power BI; ROADMAP
  `m4-windows-groundtruth` (GT-2, GT-6), DESKTOP-PLAN M13.5, the GAPS "differential testing" row
  (score 0). Done before new features, at the owner's request.
- **Numbers:** 11 suites, 2,212 cases (601 Acme measures, 747 `FORMAT`, 522 DAX function
  cases, 286 on three MIT samples, …), all equal or ledgered: 6 ledger entries (1 bug: the
  implied `ALLSELECTED` of `OFFSET`/`WINDOW`; 4 lenient; 1 float summation order), 29 volatile
  cases. 22 findings: 9 DashCraft bug classes fixed (BLANK logic, `FORMAT` rules, `MEDIAN` over
  dates, `List.Dates`, blank rows of inactive relationships, Text↔Number keys,
  `COUNTBLANK`/`DISTINCTCOUNT` and the blank row, text case variants, out-of-range `FORMAT`
  dates), 2 PBIT writer bugs fixed (`QueriesSettings` without `Version`, missing level ordinals:
  Desktop could not open our PBIT), 2 Acme model errors fixed (static plus dynamic format string,
  now `LINT018`; the reserved name `Catalog`), 6 ledger entries, 1 adapter difference
  (`INFO.VIEW.RELATIONSHIPS` lists one-to-one relationships twice), `TODAY()` calculated columns
  marked volatile, and 1 Desktop behaviour documented (template import resets a custom date
  format).
  PBIP written by DashCraft: 643/643 equal in Desktop; PBIT: 642/643. Two Acme render snapshots
  gain the "(Blank)" member Desktop shows. GAPS DAX 3.3 → 3.8.
- **Open:** Desktop's own visual queries (Performance analyzer export) and golden images need
  manual GUI steps ([reference/groundtruth/README.md](../reference/groundtruth/README.md#manual-sessions-not-yet-recorded));
  the `ALLSELECTED` shadow-context bug; GT-1, GT-4 and GT-5.

## 2026-11-04

### CI build reuse, Windows linking and test scheduling

- **What:** Added tested conservative frontend-only job selection, workspace-only compiler
  caching alongside existing dependency caches, trusted-PR dependency-cache saves, bundled
  Windows LLD with debug symbols disabled, and nextest with separate doctests. App checks
  remain on all three platforms; Rust changes still run the complete matrix and perf jobs.
- **Why:** PR #16 took about 34 minutes versus 11 for its identical merged tree with warmer
  caches. Windows spent 24m16s compiling/linking tests and about four minutes executing them.
- **Numbers:** Six change-selection regression tests; existing assertions, ignored-test policy,
  performance budgets and golden thresholds are unchanged. CI run `41927081220`, attempts 1/2
  on `d71dcc1`, passed all ten jobs both times: workflow 37m28s → 7m02s; Windows Rust
  37m14s → 6m46s, test compilation 25m46s → 1m45s. All 1,228 Windows tests and twelve
  doctests passed; two doctests remain ignored. APT was uncached and varied 10m12s → 23s:
  slow package downloads remain open. Initial cache migration can still require a full build;
  these runs demonstrate warm reuse, not a controlled old/new benchmark. Commits `b1c69b0`,
  `d71dcc1`; details and all-platform timings in [CI](ci.md#build-reuse-and-test-execution).

### Dependabot PR #16: SQLite serialization compatibility

- **What:** The two synthetic SQLite fixture builders in `pbi-vertipaq` use mutable
  connections for rusqlite 0.41.1's `serialize` API. Regenerated `THIRD_PARTY_NOTICES.md`
  for rusqlite 0.41.1 and libsqlite3-sys 0.39.1, retaining the other dependency updates.
- **Why:** All three Rust CI jobs failed with E0596 at these helpers after the dependency
  update. Production readers and existing metadata/VertiPaq assertions are unchanged.
- **Numbers:** Two binding changes fix the two compiler errors; existing tests exercise both
  fixture builders. Full cross-platform CI validates the combined head before any merge.

### Dependabot PR #14: paired SVG renderer update

- **What:** Updated resvg and usvg together from 0.48.1 to 0.49.0 and regenerated
  `THIRD_PARTY_NOTICES.md`, retaining merged PRs #13, #15 and #17.
- **Why:** Upstream clipping and text-layout fixes; the existing bundled-font, offline image
  resolver, deterministic PNG and snapshot checks stay unchanged.
- **Numbers:** Two renderer versions updated. The original PR passed all 10 CI jobs; the
  refreshed combined head is checked before merging. Optional image-level review is deferred;
  CI has no optional Power BI golden images, so it cannot establish golden SSIM.

### Dependabot PR #15: XML, ZIP and dialog maintenance updates

- **What:** `Cargo.lock` updates roxmltree 0.21.1 to 0.21.2, zip 9.0.5 to 9.2.2 and
  tauri-plugin-dialog 2.8.3 to 2.8.4, including their transitive changes; regenerated
  `THIRD_PARTY_NOTICES.md`. Based on the refreshed PR #13 head so both CI runs can run
  concurrently while checking the combined dependency graph.
- **Why:** The XML patch avoids stack overflow on deeply nested elements; the ZIP and dialog
  updates include upstream maintenance fixes. Existing container limits and tests stay unchanged.
- **Numbers:** Three direct updates; Arrow/Parquet 59.3.2 and DataFusion 55.2.0 are retained.
  The original PR passed all 10 CI jobs; the combined head is checked again before merging.

### Dependabot PR #13: Arrow and Parquet maintenance update

- **What:** `Cargo.lock` updates the Arrow/Parquet family from 59.3.1 to 59.3.2; regenerated
  `THIRD_PARTY_NOTICES.md`. Integrated current `main`, retaining the financial-function fixes,
  dual licensing and non-yanked executor dependency.
- **Why:** Upstream maintenance fixes cover IPC validity buffers, array offsets, buffer handling
  and Parquet decoding without crossing the Arrow major supported by DataFusion.
- **Numbers:** 15 crate versions updated; DataFusion stays at 55.2.0. The original PR passed all
  10 CI jobs; the refreshed head is checked again before merging.

### PR #17 CI: unsupported-function fixture and yanked dependency

- **What:** The session robustness fixture uses `UNSUPPORTED_TEST_FUNCTION()` instead of
  the now-supported `PMT`, preserving the typed-error and skipped-dependant assertions.
  `Cargo.lock` updates only `async-executor` 1.14.0 → 1.14.1; regenerated `THIRD_PARTY_NOTICES.md`.
- **Why:** The first approved full CI run exposed the stale fixture and a dependency yanked
  on November 2. Neither check is weakened or suppressed.
- **Numbers:** The session test fails before the fixture update; all four `cargo deny` checks
  pass with the patched lockfile. No other dependency version changes.

### PR #17 second review: DAX compatibility and financial numerical stability

- **Commit:** `87cc931`.
- **What:** `pbi-dax` rounds payment types before validating 0/1 and rejects non-finite or
  below-one `nper` in `PMT`/`PV`/`FV`, matching the documented DAX bounds. `PMT`/`PV`
  discount positive-rate cash flows rather than overflowing an intermediate growth factor.
  `NPER` uses separate logarithms away from a ratio of 1, preserving valid small positive
  ratios that the earlier `ln_1p`-only fix rounded to zero.
- **Why:** Second review reproduced four failures, including an `NPER` regression from
  `6fd1f3b`; the DAX reference specifies rounding and period bounds that differ from Excel.
- **Numbers:** Four new regression tests fail before the fixes and pass afterward; they cover
  rounding boundaries, invalid periods, small growth ratios, long horizons and nonzero future values.

### DAX financial functions; MIT OR Apache-2.0 (PR #17)

- **What:** `pbi-dax` `eval/functions/financial.rs`: `PMT`, `PV`, `FV`, `NPER` (`type` 0/1, `rate = 0`
  limits, `rate <= -1` and `type` validation). `(1 + rate)^nper` and the annuity factor go through
  `ln_1p`/`exp_m1`, and `NPER` through `ln_1p` of the ratio's offset from 1, so tiny non-zero rates
  (`1.0 + rate == 1.0`) match the zero-rate limits instead of failing or returning 0. Licence: `MIT OR
  Apache-2.0` (`LICENSE` → `LICENSE-MIT`, new `LICENSE-APACHE`, workspace and fuzz manifests, NOTICE,
  README, About page); the app bundles both files under `licenses/` (`tauri.conf.json` still pointed
  at the removed `LICENSE`, which broke the `dashcraft-app` build script).
- **Why:** GAPS "Function library breadth" (financial functions missing).
- **Numbers:** 294 → 298 DAX functions (275 supported, 23 partial); `financial_functions` covers
  mortgage, savings, annuity due, zero and tiny (±1e-20, 1e-12) rates and invalid arguments; the
  never-panic sweep includes the four functions. Open: `XIRR`, `XNPV` and the other financial functions.

## 2026-10-26

### App: load data on open as Power BI Desktop does

- **What:** `dashcraft-app` `DocumentSummary.onOpen` (`none` / `embedded` / `cache` / `query`,
  computed in `document_views`); `App.svelte` starts `data.refresh` from it: PBIX/ABF/Power Pivot
  embedded data (source `cache`, quiet, no cache banner), PBIP `.pbi/cache.abf` (unchanged), PBIT
  refresh from the source with a banner about the saved parameter values. PBIP without a cache,
  `.bim`, TMDL folders and remote models stay empty until Refresh. Test
  `on_open_loads_what_desktop_loads` (PBIX, PBIT, semantic model with and without a cache).
- **Why:** a PBIX opened empty until Refresh, unlike Desktop, which shows the data saved in it.
- **Numbers:** `dashcraft-app` tests 29 → 30; Revenue Opportunities.pbix loads 8 tables on open.
  Open: Desktop's "Enter parameters" prompt for templates (no parameter-editing command yet).
- **CI:** `cargo deny check advisories` failed on main for RUSTSEC-2026-0391, -0398 and -0403
  (zstd 0.13.3 / zstd-safe 7.3.0 via arrow-ipc and parquet 59.3; fixes only in zstd 0.14.3 /
  zstd-safe 8.2, out of reach under DataFusion 55.2). None is reachable (bulk API only, no
  `seekable` or `legacy` feature): ignored in `deny.toml` with reasons, to drop when Arrow
  accepts zstd 0.14.

## 2026-10-25

### DashCraft everywhere: `pbiview` removed from code and current docs

- **What:** [ADR-0010](adr/0010-dashcraft-everywhere.md) supersedes ADR-0009 §2/§4/§5. Removed the
  `pbiview` alias binary (`src/bin/pbiview.rs`, its test, the release smoke test and archive
  copy) and the `PBIVIEW_APP`, `PBIVIEW_BLOB_MAP`, `PBIVIEW_CONFIG_DIR` and `PBIVIEW_AI_API_KEY`
  fallbacks (CLI `open`, app blob map and AI settings, `pbi-ai` settings and key lookup), plus the
  legacy `pbiview-app` / `pbiview.app` lookup in `dashcraft open`. Renamed `crates/pbiview-cli` →
  `crates/dashcraft-cli` (`dashcraft_cli`), `pbiview-app` → `dashcraft-app` (`dashcraft_app_lib`),
  `pbiview-fuzz` → `dashcraft-fuzz`; `PBIVIEW_REFERENCE`/`BLESS`/`TEST_RECORD`/`ATTEST` →
  `DASHCRAFT_*`; `pbiview.*` export metadata keys and annotations → `dashcraft.*`; test helpers,
  temp-folder prefixes and the release heredoc marker. Current docs, `NOTICE`, AGENTS.md and the
  Copilot notes updated; CHANGELOG history, this log, ADR-0001 to 0009 and research notes keep
  the old name.
- **Why:** the owner asked to drop the old name: v0.2.0 is the first public release, so no user
  depends on the aliases.
- **Numbers:** 705 `pbiview` mentions in 121 files → 0 outside history files.

## 2026-10-24

### CI fix: app clippy on Linux and Windows; Dependabot majors

- **What:** `PendingOpen::set` (`app/src-tauri/src/lib.rs`) is compiled only on macOS/iOS and in
  tests, its only callers, so `dead_code` no longer fails `cargo clippy -p pbiview-app` on Linux and
  Windows. `.github/dependabot.yml` ignores arrow/parquet majors (DataFusion 55.2 is on arrow 59;
  the arrow 60 PR could not build) and TypeScript majors (`svelte-check` 4.7 peers on TS ≤ 6).
  `DirContainer` sorts its skipped entries by path like its entries: they came in `readdir`
  order, so `dir_symlinks_are_skipped_and_swaps_are_refused` failed at random on ext4.
  A root `.gitattributes` (`* text=auto eol=lf`; the CRLF fuzz seed and Microsoft's sample
  licence kept as committed; binary formats marked) gives Windows checkouts LF: the Windows
  runner checked out CRLF, which failed 13 tests in 8 targets (byte-identical TMDL/TMSL
  round-trips, the `COMMANDS.md`/`AI.md`/`ops.gen.ts` freshness checks, fixture-text edits).
- **Why:** `main` CI was red (App jobs on ubuntu-latest and windows-latest; the symlink test at
  random on Rust ubuntu-latest; Rust windows-latest from CRLF checkouts), and so was every
  Dependabot PR.

### MIT License and publication

- **What:** the project is licensed under MIT (`LICENSE`; `license = "MIT"` in the workspace and
  fuzz manifests; NOTICE, README, the About page and the installed `licenses/LICENSE`); the
  repository URL is `https://github.com/Jxck-S/dashcraft`; golden rule 17 now says the repository is
  public and every push is reviewed for internal data.
- **Why:** the owner chose MIT (the *Craft projects use MIT OR Apache-2.0) and publishes the source.

### README restructure with app screenshots

- **What:** `README.md` rewritten to a short front page (336 → 205 lines): icon, tagline, badges,
  a hero screenshot, four highlights, a screenshot grid (report interaction, Table, Model and DAX
  query views), agents, internals, get started, status, a documentation index. Five screenshots
  of the app on the synthetic Acme Outfitters example in `docs/images/` (PNG, about 600 KB in all).
  The detail moved: features, the format and visual matrices and DAX/M coverage to the new
  `docs/FEATURES.md`; build, gates and test-data rules to the new `docs/DEVELOPMENT.md`; the
  security posture to the new `docs/SECURITY.md`; releases and the v1 acceptance criteria to
  `docs/ROADMAP.md`. The gaps, milestone, performance, CLI and architecture tables were
  duplicates of `GAPS.md`, `DESKTOP-PLAN.md`, `perf.md`, `CLI.md` and `ARCHITECTURE.md` and are
  now links. Links into the old README sections (`GAPS.md`, `VISUALS.md`, `AGENTS.md`) updated.
- **Why:** the README had grown into a reference manual; the owner asked for a shorter front page
  organized like the PhotoCraft README, with a screenshot of the app.

### Release: arm64 installers, rpm, manual release with version override

- **What:** `.github/workflows/release.yml`: a manual run (`workflow_dispatch`) now releases
  the version in `Cargo.toml` or a `version` override (e.g. `0.3.0-rc.1`, marked pre-release),
  creates the tag `v<version>` on the selected commit and the draft release; `dry_run` keeps
  the old build-only behaviour. New targets on native Arm64 runners: Windows arm64 (NSIS) on
  `windows-11-arm`, Linux arm64 (AppImage, deb, rpm) on `ubuntu-22.04-arm`; rpm for Linux x64;
  CLI archives and SBOMs for both arm64 targets (bundle and CLI matrices 4 → 6).
  New `scripts/set-version.sh` sets the version in `Cargo.toml`, `Cargo.lock` and the npm
  manifests (byte-identical when the version is unchanged); the workflow uses it for an override.
  `docs/packaging.md`, `docs/ci.md` updated.
- **Why:** the owner asked for installers on the platforms PhotoCraft ships (its shared release
  workflow builds Windows x64/x86/arm64, Linux x86_64/aarch64 with rpm, Flatpak and more) and for
  a manual release trigger with a version override for release candidates. Kept: two thin macOS
  builds instead of a universal one (documented choice). Not added: Windows x86, riscv64,
  Flatpak, Arch, FreeBSD.
- **Numbers:** `actionlint` clean. Not yet run on GitHub: the first dry run will confirm the
  Arm64 runners, the arm64 AppImage and the rpm file names.

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
