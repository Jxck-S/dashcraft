# ADR-0010: DashCraft everywhere: no `pbiview` aliases or internal names

- Status: Accepted
- Date: 2026-10-25
- Deciders: the project owner
- Supersedes: [ADR-0009](0009-naming-dashcraft.md) §2 (the `pbiview` alias binary), §4 (`PBIVIEW_*` aliases of user variables) and §5 (internal names and data contracts kept as `pbiview`)
- Related: [AGENTS.md §10](../../AGENTS.md#10-naming-and-trademark-policy)

## Context

ADR-0009 renamed the product to DashCraft but kept the old name in three places: a `pbiview`
alias binary and `PBIVIEW_*` aliases for the user environment variables, so scripts written for
the old name kept working; the front-end crate names (`pbiview-cli`, `pbiview-app`) and the
developer and test variables (`PBIVIEW_REFERENCE`, `PBIVIEW_BLESS`, …); and data contracts written
into users' files (the `pbiview.*` keys of exported Parquet metadata and the `pbiview.derived*`
model annotations).

DashCraft's first public release is v0.2.0 (2026-10-25). Nobody outside the project has scripts,
settings or exported files that use the old name, so the compatibility layer protects no one and
leaves two names in the code, the CLI help and the docs.

## Decision

Use only the DashCraft name in code, configuration and current documentation:

1. **No aliases.** The `pbiview` binary is removed. `PBIVIEW_APP`, `PBIVIEW_BLOB_MAP`,
   `PBIVIEW_CONFIG_DIR` and `PBIVIEW_AI_API_KEY` are no longer read; only the `DASHCRAFT_*` names
   are. `dashcraft open` no longer looks for `pbiview-app` or `pbiview.app`.
2. **Front-end crates:** `pbiview-cli` → `dashcraft-cli` (folder `crates/dashcraft-cli`, library
   `dashcraft_cli`), `pbiview-app` → `dashcraft-app` (library `dashcraft_app_lib`), the fuzz crate
   `pbiview-fuzz` → `dashcraft-fuzz`. The library crates keep the `pbi-*` prefix, which names the
   file format, not the product.
3. **Developer and test variables:** `DASHCRAFT_REFERENCE`, `DASHCRAFT_BLESS`,
   `DASHCRAFT_TEST_RECORD`, and the repository variable `DASHCRAFT_ATTEST`.
4. **Data contracts:** export metadata keys and model annotations are `dashcraft.*`
   (`dashcraft.schemaVersion`, `dashcraft.table`, `dashcraft.derived`, …). Files written with the
   `pbiview.*` keys are not read back specially.
5. **History is kept:** CHANGELOG sections, the dev log, ADR-0001 to ADR-0009 and the research
   notes keep the name they were written with.

## Alternatives considered

| Option | Why not |
|---|---|
| Keep the aliases until 1.0 with a deprecation warning | Protects no existing users (none had the old name), and keeps two names in help, docs and tests for months |
| Rename only user-visible names, keep `pbiview-*` crates | Contributors and agents still see two names in every build command (`-p pbiview-cli`) and path |
| Read both `pbiview.*` and `dashcraft.*` metadata keys | No file with the old keys exists outside the repository's own tests |

## Consequences

- One name everywhere: `cargo build -p dashcraft-cli`, `cargo test -p dashcraft-app`, `DASHCRAFT_*`.
- Anyone who built from source before 2026-10-25 and set a `PBIVIEW_*` variable must rename it,
  and the repository variable `PBIVIEW_ATTEST` (if set) becomes `DASHCRAFT_ATTEST`.
- The decision stays reversible as ADR-0009 describes; a future rename repeats this change set.
