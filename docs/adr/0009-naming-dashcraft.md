# ADR-0009: Public product name DashCraft (formerly pbiview)

- Status: Accepted (the name is chosen "for now"; reversible, see Consequences). §2, §4 and §5 superseded by [ADR-0010](0010-dashcraft-everywhere.md)
- Date: 2026-10-22
- Deciders: the project owner (name), DashCraft maintainers (implementation)
- Related: [ADR-0001](0001-ui-tauri-svelte.md) (app identifiers), [ADR-0007](0007-command-registry.md) (CLI, MCP), [ADR-0008](0008-ai-layer.md) (keychain service, `DASHCRAFT_*` variables); research: [artcraft-infra-review.md](../research/artcraft-infra-review.md) §8 ("Craft" naming and brand); [AGENTS.md §10](../../AGENTS.md#10-naming-and-trademark-policy)

## Context

The project started as `pbiview`: the name of the CLI binary, the desktop app (`pbiview-app`, bundle `pbiview.app`), the environment variables (`PBIVIEW_*`), the MCP server and every document. A public release needs a product name that:

- does not contain "Power BI" or suggest a Microsoft product (Microsoft's trademarks may only be used nominatively, to say which files are read);
- is short, pronounceable and usable as a command name;
- does not clash with an existing product in the same space.

The owner chose **DashCraft**. A quick search before adopting it found:

- **The name is already in use.** dashcraft.app is a dashboard tool for Tableau (the same "BI dashboards" space), and "DashCraft AI" is used by at least one AI dashboard product. Neither is affiliated with this project. No trademark search has been done; the risk is confusion with those products, and a takedown or rename request if one of them holds a registered mark.
- **"\*Craft" brand risk.** storytold's ArtCraft family (PhotoCraft, GridCraft, DeckCraft, …; studied for patterns only, never copied) asserts the `{Function}Craft` naming convention and the ArtCraft brand and does not license them to third parties; its brand terms say other products must never suggest that the ArtCraft team made, sponsors or endorses them ([artcraft-infra-review.md §8](../research/artcraft-infra-review.md#8-recommendations-for-pbiview--dashcraft)). A "DashCraft" with a README or badges styled like theirs would invite confusion.

## Decision

1. **Product name: DashCraft** everywhere a user sees the product: README title, app window title, `productName`, About and Backstage, CLI help and messages, docs, the MCP server, release artifacts. The README says "formerly pbiview" once. The product is described as a tool that "opens Power BI files"; "Power BI" never appears in the product name, the app name or the icon.
2. **CLI: binary `dashcraft`, with `pbiview` kept as an alias binary.** Both are built from the `pbiview-cli` package (`src/lib.rs` holds the program as `run_main()`; `src/bin/dashcraft.rs` and `src/bin/pbiview.rs` call it) and both ship in the CLI archives. Usage, `--version` and messages use the name the program was invoked as (`pbiview: error: …` under the alias, `dashcraft: error: …` otherwise). JSON outputs, schemas, command ids and exit codes do not change.
3. **Desktop app:** `productName` "DashCraft", `mainBinaryName` and Cargo binary `dashcraft-app`, window title "DashCraft", bundle identifier **`io.github.jxck-s.dashcraft`**. Tauri names the app config and data folders after the identifier; the app is not released yet, so it has no settings to migrate from earlier identifiers. The Windows MSI keeps its `upgradeCode`, so installing DashCraft replaces an installed pbiview MSI; NSIS, macOS and Linux installs of pbiview are separate apps that can be removed by hand. Linux MIME types become `application/x-dashcraft-*` (`scripts/packaging/linux/dashcraft-mime.xml`, `dashcraft.desktop.hbs`). `dashcraft open` looks for `dashcraft-app` (bundle `DashCraft.app`) first, then the legacy `pbiview-app` / `pbiview.app`.
4. **Environment variables** that users set get the `DASHCRAFT_` prefix; the `PBIVIEW_` names stay accepted as aliases and the new name wins when both are set: `DASHCRAFT_APP` (`PBIVIEW_APP`, CLI `open`), `DASHCRAFT_BLOB_MAP` (`PBIVIEW_BLOB_MAP`, app), `DASHCRAFT_CONFIG_DIR` (`PBIVIEW_CONFIG_DIR`) and `DASHCRAFT_AI_API_KEY` (`PBIVIEW_AI_API_KEY`, AI layer). Developer and CI variables keep their names and are documented as internal: `PBIVIEW_REFERENCE`, `PBIVIEW_BLESS`, `PBIVIEW_TEST_RECORD`, the `PBIVIEW_ATTEST` repository variable and the `__PBIVIEW_EOF__` workflow delimiter.
5. **Unchanged internals:** crate and package names stay `pbi-*` / `pbiview-*` (`pbiview-cli`, `pbiview-app`, `pbiview_app_lib`), as do Parquet field-metadata keys and other data contracts. The OS keychain service is already `dashcraft` (ADR-0008).
6. **MCP:** `serverInfo.name` is `dashcraft` (title "DashCraft: Power BI files, offline") and the tool `_meta` keys use the `dashcraft/` prefix (`dashcraft/commandId`, `dashcraft/category`); both were unreleased.
7. **Legal notice:** a root [`NOTICE`](../../NOTICE) file, linked from the README and summarised in the app's About page and installed with it (`licenses/NOTICE`), states that DashCraft is not affiliated with, endorsed or sponsored by Microsoft; that "Power BI" and "Microsoft" are Microsoft trademarks used only to describe file compatibility; that DashCraft is not affiliated with storytold/ArtCraft or with other products named "DashCraft" (dashcraft.app's Tableau tool, DashCraft AI); and points to `THIRD_PARTY_NOTICES.md` for bundled third-party assets. No ArtCraft-style badges, logos or README layout.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Keep `pbiview` | Reads as "Power BI viewer": leans on Microsoft's mark in the product name, which the trademark policy rules out for a public product. |
| **Kestrel BI** | Distinctive and unrelated to the *Craft family; "BI" is generic. Not chosen by the owner; the first fallback if DashCraft has to go. |
| **Rook BI** | Short and command-friendly (`rook`); a crowded word in software (Rook, the Kubernetes storage operator). |
| **Glasswing BI** | Evokes "see-through" (a read-only viewer); long as a command name. |
| **Daxwell** | Plays on DAX; ties the name to one Microsoft language and may read as affiliated with it. |
| **DashCraft** (chosen "for now") | Descriptive (dashboards + craft), short, a usable command. Known risks: existing products named DashCraft and the "\*Craft" brand family (Context); mitigated by the `NOTICE` disclaimer and by never styling the project like ArtCraft. |

## Consequences

- Users type `dashcraft`; scripts that call `pbiview` keep working, and so do `PBIVIEW_APP` and `PBIVIEW_BLOB_MAP`. The CLI's JSON outputs, `inspect --json` snapshots, command ids and schemas are unchanged.
- The app had no release before the identifier was set, so there are no app settings (File → Recent, `ai.json`) to carry over.
- Documents describe DashCraft; the CHANGELOG and DEVLOG history and ADR-0001 to ADR-0008 keep the name they were written with.
- **The decision is reversible.** A later rename (for example to Kestrel BI) repeats this change set: the product strings (README, `tauri.conf.json` `productName` / window title / descriptions, About and Backstage, app strings, CLI help, docs, `NOTICE`); a new CLI binary in `crates/pbiview-cli/src/bin/` with `dashcraft` and `pbiview` kept as aliases; `mainBinaryName`, the app's Cargo `[[bin]]` and `dashcraft open`'s lookup list; a new identifier with a one-time migration of the settings files (File → Recent, `ai.json`) from the `io.github.jxck-s.dashcraft` folders; new environment-variable names with the current ones as aliases; the MCP server name and `_meta` prefix; Linux MIME types and packaging files; release workflow artifact names; the keychain service (with a fallback read of `dashcraft` entries, or users re-run `ai login`); and a new ADR superseding this one. Internal crate names never need to change.
- Open: a trademark search before the first public release, and a review of the name if any "DashCraft" product objects.
