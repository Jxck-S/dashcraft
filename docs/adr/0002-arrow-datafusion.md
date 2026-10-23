# ADR-0002: Data engine on Apache Arrow and DataFusion

- Status: Accepted (see the Phase 2 status note under Consequences)
- Date: 2026-10-21
- Deciders: pbiview maintainers
- Related: [ADR-0004](0004-desktop-ready-architecture.md)

## Context

`pbiview` needs a columnar in-memory store and a query layer for:

- Tables loaded from M partitions (CSV, Azure Blob), decoded VertiPaq segments (PBIX, `.abf`), and calculated tables.
- The data grid: sort, page and compute column statistics.
- Exports to CSV and Parquet.
- A **real DAX engine** (filter context, row context, context transition, blank semantics, relationship-based filter propagation, including bidirectional). Phase 1 already needs it for calculated columns and tables, and the KPI measures. Phase 2 needs it for every visual query.

The spec allows DataFusion or Polars on top of Arrow (`arrow-rs`).

## Decision

- **Storage:** Apache Arrow (`arrow` 59.2) record batches and arrays are the in-memory format for every table, whatever its source. Dictionary arrays represent VertiPaq-style dictionary-encoded columns.
- **Query layer:** Apache DataFusion 55.2 (`default-features = false`, with only the expression feature sets we use). `pbi-engine` owns a `SessionContext` and registers each model table as a `TableProvider`.
- **DAX on DataFusion:** `pbi-dax` owns DAX semantics. It does not translate DAX to SQL text. Instead it follows the Analysis Services split between a formula engine and a storage engine:
  - The DAX evaluator (formula engine) resolves filter context, row context, context transition and `BLANK`.
  - It lowers storage requests (scan, filter, semi-join over relationships, group by, aggregate) into DataFusion logical plans, which run through `pbi-engine` (storage engine).
- **Versions:** `arrow`/`parquet` 59.2 are pinned to the version DataFusion 55.2 uses. They are upgraded together. DataFusion needs Rust 1.94 or newer, so the toolchain is pinned to 1.99.1 (`rust-toolchain.toml`).

### Why DataFusion is extensible enough for a DAX engine

| Extension point | Use in pbiview |
|---|---|
| `TableProvider` | Expose in-memory model tables, and later VertiPaq-backed lazy tables, with projection and filter pushdown. |
| `ScalarUDF` / `AggregateUDF` / `WindowUDF` | DAX-specific functions with `BLANK`-aware semantics (`DIVIDE`, `DISTINCTCOUNTNOBLANK`, `FORMAT`, text and date functions). |
| `UserDefinedLogicalNode` + `ExtensionPlanner` | Custom plan nodes when DAX needs operators SQL lacks (for example relationship-path filter propagation or `SUMMARIZE` with rollup semantics). |
| `OptimizerRule` / `AnalyzerRule` | DAX-aware rewrites (filter pushdown across relationships, removing redundant `ALL`, caching of repeated sub-plans). |
| `CatalogProvider` / `SchemaProvider` | One catalog per open document. Table names follow the model, including quoted names with spaces. |
| Async `ExecutionPlan` streams | Execution is async and streaming. Dropping the stream cancels the query, which fits the async + cancellation API in [ADR-0004](0004-desktop-ready-architecture.md). |
| SQL front end | Debugging aid only (for example a developer console). It is not part of the DAX path. |

## Alternatives considered

| Option | Assessment |
|---|---|
| **Polars** | Very fast DataFrame library with a good lazy API. However: (1) it uses its own Arrow implementation (`polars-arrow`), not `arrow-rs`, so exchanging data with Arrow, Parquet and VertiPaq decoders needs C Data Interface conversion; (2) its Rust API is not semver-stable and changes often; (3) extension points are aimed at expression plugins (mostly through Python), not custom logical plans, optimizer rules or table providers. It suits data wrangling better than hosting a second query language. |
| Hand-written evaluator over Arrow compute kernels only | Smallest dependency, and fine for the fixture's size. But every join, group-by, sort and spill strategy would be ours to write and optimize. We still use `arrow` compute kernels directly for simple cases (single-column aggregates, grid sorting). |
| DuckDB (embedded) | Strong engine with an Arrow interface, but it is a C++ dependency with FFI (`unsafe`, larger build, harder fuzzing), and SQL is the only extension surface. |
| SQLite | Row store. Not suited to columnar analytics. (`rusqlite` is still used to read `metadata.sqlitedb` inside ABF backups.) |

## Consequences

- One memory format end to end: M results, VertiPaq decode, DAX intermediate tables, grid pages (Arrow IPC) and Parquet export share Arrow arrays without copying.
- Compile times and binary size grow. Default features are off, and only needed DataFusion features are enabled.
- Arrow and DataFusion must be upgraded in lockstep. The root `Cargo.toml` is the only place versions change.
- SQL `NULL` semantics differ from DAX `BLANK` (for example `BLANK + 1 = 1`, and `BLANK = 0` is true). The DAX evaluator and its UDFs own these rules. DataFusion operators are never relied on for `BLANK` semantics.
- The fixture model is small, so DataFusion planning overhead dominates. Hot paths may use Arrow kernels directly. The API stays the same.

> Status note (Phase 2): as anticipated above, the storage engine (`pbi_engine::ops`) computes visible row sets, distinct values, group-bys and aggregates directly over the Arrow arrays, and `pbi-dax` calls it without building DataFusion plans. DataFusion is used for SQL: the explorer grid's filter expressions and `Session::sql`. The extension points listed above remain the path if query sizes outgrow this direct path.
