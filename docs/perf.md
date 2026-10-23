# Performance budgets

`cargo xtask perf` measures a fixed set of scenarios in a release build and compares them
with numeric budgets (`perf/budgets.toml`) and with recorded numbers for the same machine
(`perf/baseline.json`), so a slowdown shows up as a number in review instead of as lag
reported by a user. The pattern (budgets file + per-machine baseline + `xtask` runner) comes
from the *Craft apps; see [photocraft-review.md](research/photocraft-review.md) §2
"Performance techniques" and §4, and
[craft-media-apps-review.md](research/craft-media-apps-review.md) recommendation 4.

## Quick start

```sh
cargo xtask perf --list                 # scenarios, budgets, data source
cargo xtask perf                        # all scenarios, full iteration counts (~3 min on an M4 Pro)
cargo xtask perf --quick                # fewer iterations (what CI runs)
cargo xtask perf --scenario P01         # one scenario (id or name; repeat or comma-separate)
cargo xtask perf --update-baseline      # record this machine's numbers in perf/baseline.json
cargo xtask perf --json > perf.json     # JSON report on stdout, table on stderr
cargo xtask perf --no-fail              # report, but exit 0
```

Other options: `--warmup N` and `--iterations N` override every scenario's counts,
`--reference DIR` points at another fixture root (default `$PBIVIEW_REFERENCE`, else
`reference/`), `--locked` is passed to `cargo build`. Every run writes
`target/perf/report.json` (all statistics and the scenarios' `info`) and
`target/perf/report.md` (the table, used as the CI job summary).

Exit status: `0` when nothing failed, `1` when a scenario errored, exceeded its `max_ms` or
regressed, `2` for usage or build errors.

## How it works

1. **Scenarios** are listed in `perf/budgets.toml`. Each names the crate and the `perf_*`
   example that implements it, so a scenario lives next to the code it measures and only
   uses public APIs:

   | Example | Scenarios |
   |---|---|
   | `crates/pbi-render/examples/perf_render.rs` | P01, P02, P03, P07, P08 |
   | `crates/pbi-dax/examples/perf_dax.rs` | P04 |
   | `crates/pbi-m/examples/perf_m.rs` | P05 |
   | `crates/pbi-vertipaq/examples/perf_vertipaq.rs` | P06 |

2. **Build.** `xtask` builds all perf examples with one `cargo build --release` (same
   feature unification on every run; the shared `target/release`). `xtask` itself depends on
   no workspace crate, so `cargo xtask layers` stays fast.
3. **Run.** Each scenario runs as
   `<example> --scenario <ID> --warmup <N> --iterations <N> --reference <DIR>`. Setup (data
   generation, loading, the first query) is not timed. The example runs `warmup` untimed and
   `iterations` timed iterations and prints one JSON line with the samples and an `info`
   object (sizes, sub-timings; never data values). The loop and the line format are in
   `perf/harness.rs`, which every example includes with `#[path]`. Scenarios with
   `per_process = true` (P02) run one process per sample, because the shaping and grid-layout
   caches are process-wide and a second render in the same process is no longer cold.
4. **Statistics.** Median and nearest-rank p95 (also min, max, mean) per scenario. `metric`
   selects which one is judged: p95 for scroll steps (a slow step is visible jank), median
   elsewhere.
5. **Verdict**, per scenario:

   | Status | When | Fails the run |
   |---|---|---|
   | `ok` | metric ≤ `target_ms` | no |
   | `over target` | `target_ms` < metric ≤ `max_ms` | no |
   | `FAIL (over max)` | metric > `max_ms` | yes |
   | `FAIL (regressed)` | metric > baseline × (1 + `regression_pct`/100) **and** metric − baseline > `regression_floor_ms` (20 % and 1 ms) | yes (not with `--update-baseline`) |
   | `skipped` | the scenario's data is missing (`SKIP: <path> missing`) | no |
   | `ERROR` | the example failed or printed no result | yes |

   `target_ms` is the goal (what a user should get); `max_ms` is the ceiling beyond which the
   feature counts as broken. The noise floor keeps sub-millisecond scenarios from "regressing"
   on scheduler jitter.
6. **Baseline.** `perf/baseline.json` holds results per machine, keyed by OS, architecture
   and CPU model (`macos-aarch64-apple-m4-pro`), with the OS version, core count, `rustc`
   version and date. Runs on a machine without an entry skip the regression check (CI runners).

## Scenarios

All synthetic data is generated in code with a fixed seed, so every run measures the same
input. "Committed" scenarios use the committed Acme Outfitters example or Microsoft sample reports
and run on a fresh clone; a "private" scenario would use gitignored local data and print `SKIP`
without it.

| ID | Name | What one iteration does | Data | Metric | Target / max (ms) |
|---|---|---|---|---|---|
| P01 | `table-scroll-step` | `render_interactive_page` + `to_svg` of a page holding one 50 000-row × 8-column table (6 columns + 2 summed; the query keeps the app's default 30 000-row cap), scrolled 40 px further each step; warm caches, as in the app | synthetic | p95 | 16 / 33 |
| P02 | `table-page-cold` | first render of the P01 page in a fresh process: `query_page_data` (`SUMMARIZECOLUMNS` over 50 000 rows) + render + SVG | synthetic | median | 1000 / 3000 |
| P03 | `example-page-no-data` | page 1 (Executive Summary, 14 visuals) of the Overview report of the committed Acme Outfitters example (`reference/examples/acme-outfitters/AcmeOutfitters.pbip`) without a refresh: query of empty tables + layout + render + SVG | committed | median | 16 / 50 |
| P04 | `dax-measures-1m` | 5 measures over a 1 000 000-row fact table related to a 1 000-row dimension, each once in an empty filter context: `COUNTROWS`, `SUM`, `CALCULATE` with a dimension filter, `CALCULATE` with two fact filters, `SUMX` | synthetic | median | 100 / 500 |
| P05 | `m-csv-100k` | M partition over a 100 000-row (7 MB) CSV from an in-memory `BlobResolver`: `Csv.Document`, `Table.PromoteHeaders`, `Table.TransformColumnTypes`, `Table.SelectRows`, `Table.AddColumn`, then Arrow | synthetic | median | 250 / 1000 |
| P06 | `pbix-data-open` | `open_pbix_data` on the `DataModel` of the committed Microsoft sample `reference/samples/microsoft/Revenue Opportunities.pbix` (MIT): XPress9 → ABF → `metadata.sqlitedb` → VertiPaq → Arrow | committed | median | 20 / 100 |
| P07 | `busy-page-svg` | `to_svg` of a rendered synthetic dashboard: 4 cards, a slicer, column, line (730 points) and pie charts, a matrix, a 5 000-row table | synthetic | median | 4 / 16 |
| P08 | `example-table-scroll-step` | P01 on the example's data: a 40 px scroll step (render + SVG) of the table (29 rows) on the Customer Detail page of the Acme Outfitters Online report, after an offline refresh from its committed CSVs (`blob-map.json`) | committed | p95 | 16 / 33 |

Reproduce one scenario by hand (prints its JSON line; progress on stderr):

```sh
cargo run --release -p pbi-render   --example perf_render   -- --scenario P01 --iterations 60
cargo run --release -p pbi-render   --example perf_render   -- --scenario P02 --iterations 1
cargo run --release -p pbi-dax      --example perf_dax      -- --scenario P04
cargo run --release -p pbi-m        --example perf_m        -- --scenario P05
cargo run --release -p pbi-vertipaq --example perf_vertipaq -- --scenario P06
```

`crates/pbi-render/examples/bench_scroll.rs` remains the ad-hoc tool for scrolling any PBIP
(`--report`, `--page`, `--steps`), e.g. the example:

```sh
E=reference/examples/acme-outfitters
cargo run --release -p pbi-render --example bench_scroll -- $E/AcmeOutfitters.pbip \
  --blob-map $E/blob-map.json --report Online --page "Customer Detail"
```

## Current numbers

Baseline on an Apple M4 Pro (14 cores), macOS 26.5, rustc 1.99.1, full iteration counts,
recorded 2026-10-22 after the render cache and the DAX fast paths (idle machine). P03, P06 and
P08 moved to the committed example and sample on 2026-10-23; their entries were recorded then
(`cargo xtask perf --update-baseline --scenario P03,P06,P08`, on a machine that was not fully
idle; re-record them with the next full baseline):

| ID | Result (ms) | Target / max | Status | Where the time goes (`info`) |
|---|--:|---|---|---|
| P01 | 0.68 (p95) | 16 / 33 | ok | render 0.13, SVG 0.51 |
| P02 | 97 | 1000 / 3000 | ok | query 47, render 49, SVG 0.5 |
| P03 | 0.73 | 16 / 50 | ok | 14 visuals, 23 KB of SVG |
| P04 | 22 | 100 / 500 | ok | `SUMX` 19; `SUM` 1.2, `CALCULATE` 0.5–1.4, `COUNTROWS` 0.04 |
| P05 | 141 | 250 / 1000 | ok | 74 948 result rows |
| P06 | 4.7 | 20 / 100 | ok | 205 KB `DataModel`, 9 tables, 69 columns |
| P07 | 1.5 | 4 / 16 | ok | 186 KB of SVG, 866 hit regions |
| P08 | 0.35 (p95) | 16 / 33 | ok | 29 rows; render 0.06, SVG 0.25, 37 KB |

### Fixed misses

The first baseline (same day) missed three budgets; the fixes are kept here because they show
what the scenarios catch.

- **P02 was quadratic in the row count** (13.6 s at 50 000 rows): each `SUMMARIZECOLUMNS`
  group built its own filter context over the whole fact table. Plain aggregations now group
  the fact table's visible rows in one pass (`pbi-dax` fast paths, `pbi-engine`
  `group_index`/`aggregate_groups`), and the general path uses per-column value indexes for
  `IN` filters. Differential tests compare both paths (`crates/pbi-dax/src/eval/fast/tests.rs`).
- **P01, a scroll step was O(rows)** (25.6 ms at 50 000 rows): every render rebuilt the grid
  model and hashed every row. Visuals are now cached by a content hash, page data carries an
  id so its hash is computed once per query, and the grid model and layout are reused across
  scroll steps (`pbi-render` `visuals/cache.rs`, `fingerprint.rs`).
- **P04, `SUMX` was interpreted row by row** (111 ms over 1 M rows): simple column arithmetic
  inside the iterators is compiled once and evaluated over the column dictionaries
  (about 19 ns/row).

## Changing scenarios and budgets

- **New scenario:** add a `[[scenario]]` (next free `Pxx` id, unique kebab-case name, a
  description that says exactly what is timed) and implement it in the `perf_*` example of
  the crate it measures (create `crates/<crate>/examples/perf_<crate>.rs` with
  `#[path = "../../../perf/harness.rs"] mod harness;` if there is none). Prefer synthetic
  data or the committed example so CI runs it; a scenario that needs optional data returns
  `Outcome::missing(path)` when the data is absent. Then record its baseline.
- **Budgets** describe what users should get, not what the code does today: don't raise a
  `target_ms` to turn a scenario green. Lower `max_ms` when a fix lands, so the gain is kept.
- **Baseline:** record with a full run (`cargo xtask perf --update-baseline`) on an otherwise
  idle machine, after a change that is meant to move the numbers (or when adding a scenario or
  a machine), and commit `perf/baseline.json` with that change. `--scenario` limits the update
  to those scenarios. The file holds timings only.
- **Data:** never commit data or reports derived from private data (AGENTS.md rules 2 and 3). `info`
  carries sizes and sub-timings only; `target/perf/` is not committed.

## CI

The `perf` job of `.github/workflows/ci.yml` runs `cargo xtask perf --quick --no-fail
--locked` on `ubuntu-latest`, appends `report.md` to the job summary and uploads
`target/perf/` as the `perf-report` artifact. It is informational (`continue-on-error`):
shared runners are slower and noisier than the baseline machine and have no baseline entry;
scenarios marked `private` would be skipped there. Gate locally against the baseline before merging
performance-sensitive changes. See [ci.md](ci.md).
