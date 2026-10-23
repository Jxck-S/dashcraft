# Copilot instructions for DashCraft

**Read [`AGENTS.md`](../AGENTS.md) first in every session.** It is the canonical guide for every
agent (Copilot, Claude, Codex, …) and for people: project overview, layer map, golden rules,
parallel-agent protocol, build/test commands, fixtures and expected values, how to pick work,
definition of done, dev log, naming and trademark policy. This file only adds Copilot-specific
notes; if the two disagree, `AGENTS.md` wins (and fix the disagreement).

## Where to find what

| Topic | Section |
|---|---|
| What DashCraft is (internal crate names still say `pbi-*` / `pbiview-*`), phases and status | [AGENTS.md §1](../AGENTS.md#1-what-this-is), [docs/ROADMAP.md](../docs/ROADMAP.md) |
| Crates, layers (`cargo xtask layers`), dependency edges, where files live | [AGENTS.md §2](../AGENTS.md#2-layer-map-and-where-things-live) |
| Golden rules (clean-room, no real or internal report data, private data, credentials, offline, no execution, no panics, registry-first, layers, workspace deps, goldens, perf, determinism) | [AGENTS.md §3](../AGENTS.md#3-golden-rules) |
| Architecture decisions (ADR-0001 … 0009), crate layers and dependency edges | [AGENTS.md §4](../AGENTS.md#4-architecture-decisions-summary), [docs/adr/](../docs/adr/README.md) |
| Coding rules (errors, unknown fields, display names, unsupported M/DAX, async) | [AGENTS.md §5](../AGENTS.md#5-coding-rules) |
| Working next to other agents; disk hygiene | [AGENTS.md §6](../AGENTS.md#6-parallel-agent-protocol) |
| Validation commands, required test kinds, fixtures and expected values | [AGENTS.md §7](../AGENTS.md#7-build-test-validate) |
| Picking work, definition of done, staged-file scan | [AGENTS.md §8](../AGENTS.md#8-picking-work-and-definition-of-done), [docs/GAPS.md](../docs/GAPS.md) |
| Dev log | [AGENTS.md §9](../AGENTS.md#9-dev-log), [docs/DEVLOG.md](../docs/DEVLOG.md) |
| Naming (DashCraft) and trademarks | [AGENTS.md §10](../AGENTS.md#10-naming-and-trademark-policy) |
| Commits and the trailer | [AGENTS.md §11](../AGENTS.md#11-commits) |

## Copilot-specific notes

- **Copilot CLI sessions** keep notes and artifacts in the session-state folder outside the
  repository. Some of those folders contain an `azpbi/` az-login config or MSAL token caches:
  never read them into context, copy them or reference their contents (golden rule 4). Scratch
  output goes under `target/` (for example `target/tmp/`), never the system temp directory and
  never the repository tree.
- **Copilot cloud agent and code review** run on a fresh clone without the optional gitignored
  data (golden images, `reference/samples/downloads/`; AGENTS.md golden rule 3). Tests that
  need it print `SKIP: <file> missing` and pass; that is expected, not a failure. Tests use only
  the synthetic Acme Outfitters example and the licensed public samples (golden rule 2); never
  try to obtain other data or any credential, and never add it to a branch.
- **Sub-agents and parallel waves**: give each sub-agent the files it owns and the gates of
  AGENTS.md §7 for its crate (`cargo clippy -p <crate> --all-targets --no-deps -- -D warnings`,
  `cargo test -p <crate>`); the orchestrator alone edits the root `Cargo.toml`
  `[workspace.dependencies]` and commits each wave.
- **Reviewing a change**, check first for: internal data or credentials in the diff (golden
  rules 2 to 4: real report names, numbers, screenshots or URLs; gitignored paths); new
  `unwrap`/`expect`/indexing on untrusted input; an upward or new cross-layer dependency; a capability added outside `pbi-ops`; a lowered SSIM
  threshold or weakened expected-value assert; perf budgets loosened; missing tests, CHANGELOG
  `[Unreleased]` line, `docs/GAPS.md` row or `docs/DEVLOG.md` entry.
- **Public repository rule** (golden rule 17): the public repository is
  `github.com/Jxck-S/dashcraft`. Nothing from this repository goes there without a review pass.
- **Commit trailer** for every commit Copilot authors:

```
Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>
```
