# Architecture decision records

| ADR | Title | Status |
|---|---|---|
| [0001](0001-ui-tauri-svelte.md) | Desktop UI with Tauri 2, Svelte 5 and TypeScript | Accepted |
| [0002](0002-arrow-datafusion.md) | Data engine on Apache Arrow and DataFusion | Accepted |
| [0003](0003-vertipaq-xpress9-cleanroom.md) | Clean-room XPress9, ABF and VertiPaq decoding (with license research) | Accepted (legal review pending on UNVERIFIED items) |
| [0004](0004-desktop-ready-architecture.md) | Desktop-ready architecture | Accepted |
| [0005](0005-rust-svg-renderer.md) | Report rendering in Rust (DataView → scene graph → SVG) | Accepted (§6 interaction superseded by ADR-0006) |
| [0006](0006-interaction-state.md) | Report interaction state lives outside the document | Accepted |
| [0007](0007-command-registry.md) | Command registry (pbi-ops): one typed command catalog for CLI, RPC, app and agents | Accepted |
| [0008](0008-ai-layer.md) | Opt-in AI layer (pbi-ai) as a registry plugin: capability contract, providers, consent, keychain-only keys | Accepted |
| [0009](0009-naming-dashcraft.md) | Public product name DashCraft (formerly pbiview): `dashcraft` binary with a `pbiview` alias, `DASHCRAFT_*` variables, app identifier `io.github.jxck-s.dashcraft`, `NOTICE` | Accepted (name chosen "for now"; reversible) |

New ADRs use the next number and the sections Status, Context, Decision, Alternatives considered, Consequences. To change a decision, write a new ADR and mark the old one "Superseded by ADR-NNNN". Don't rewrite history.
