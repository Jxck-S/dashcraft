//! `cargo xtask <command>`: repository automation.
//!
//! * `layers`: checks that crate dependencies point down the architecture layers
//!   ([`layers`]).
//! * `perf`: runs the performance scenarios in release mode and compares them with
//!   `perf/budgets.toml` and `perf/baseline.json` ([`perf`]).
//!
//! See `docs/perf.md` and `docs/ci.md`.

mod layers;
mod perf;
mod util;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  layers   check crate dependency direction (architecture layers, UI-toolkit isolation)
  perf     run performance scenarios against perf/budgets.toml (see `cargo xtask perf --help`)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("layers") => layers::main(rest),
        Some("perf") => perf::main(rest),
        Some("help" | "-h" | "--help") | None => {
            print!("{USAGE}");
            Ok(true)
        }
        Some(other) => Err(anyhow::anyhow!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
