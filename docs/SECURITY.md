# Security posture

How DashCraft treats untrusted files, agents and network access. The rules contributors follow
are the [golden rules in AGENTS.md](../AGENTS.md#3-golden-rules); fuzzing is described in
[ci.md](ci.md#fuzz-targets).

- Every input file is **untrusted**. Parsers return errors and never panic; the workspace denies `unsafe` code. ZIP readers cap total size, entry count and compression ratio and refuse absolute, `..` and drive-letter paths (zip-slip); folders are walked without following symlinks. 14 `cargo-fuzz` targets run nightly.
- **No script execution:** R/Python visuals, `.pbiviz` custom visuals and `.mez`/`.pqx` connectors are listed or drawn as placeholders, never run. `SecurityBindings` is opaque. M code cannot read files or the network.
- **Offline-first, no telemetry.** Blob sources resolve only through a local `blob-map.json`; Web URL buttons are shown, never opened.
- **AI strictly opt-in:** `pbi-ai` is the only crate with network access (`cargo xtask layers` keeps HTTP/TLS and keychain crates inside it and its front ends); it sends nothing without a recorded consent per provider, capability and data class, and keys live only in the OS keychain.
- **Path policy for agents:** `serve --read-root/--write-root` and `mcp` confine every path to read and write roots (`..` and symlink escapes refused; `mcp` refuses writes without `--write-root`); agent callers reach only agent-safe commands.
- **Secrets are redacted** from metadata parts and connection strings before they are stored or displayed.
- **App hardening:** strict CSP with no remote origins, minimal Tauri capabilities (no file system, shell or HTTP plugins), report SVG sanitized before insertion. The app writes only its recent-files list, its AI settings (`ai.json`, no keys) and the PBIP/PBIT files you export.
- **Exports** can neutralize spreadsheet formulas (`--excel-safe`), are written atomically and never overwrite without `--force`.
