//! Measurement loop and result protocol shared by the `perf_*` examples that implement the
//! scenarios of `perf/budgets.toml` (run them with `cargo xtask perf`; see `docs/perf.md`).
//!
//! Each example includes this file with
//! `#[path = "../../../perf/harness.rs"] mod harness;` so all of them speak the same protocol
//! without a shared crate. It has no dependencies.
//!
//! Protocol: `<example> --scenario <ID> [--warmup N] [--iterations N] [--reference DIR]`
//! (`--list` prints the scenario ids) prints one JSON line per scenario on stdout:
//!
//! ```text
//! {"id":"P01","status":"ok","samples_ms":[5.1,4.9],"info":{"rows":50000}}
//! {"id":"P06","status":"skip","reason":"SKIP: reference/samples/microsoft/Revenue Opportunities.pbix missing"}
//! {"id":"P03","status":"error","reason":"..."}
//! ```
//!
//! Progress goes to stderr. `info` carries sizes and sub-timings only, never data values.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// Command-line options of a perf example.
#[derive(Debug, Clone)]
pub struct Args {
    pub scenarios: Vec<String>,
    pub warmup: usize,
    pub iterations: usize,
    /// Fixture root (`reference/`).
    pub reference: PathBuf,
    pub list: bool,
}

impl Args {
    fn parse(manifest_dir: &str) -> Result<Self, String> {
        let mut args = Self {
            scenarios: Vec::new(),
            warmup: 2,
            iterations: 10,
            reference: std::env::var_os("PBIVIEW_REFERENCE")
                .map(PathBuf::from)
                .unwrap_or_else(|| Path::new(manifest_dir).join("../../reference")),
            list: false,
        };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            let mut value = || it.next().ok_or_else(|| format!("`{a}` needs a value"));
            match a.as_str() {
                "--scenario" => args.scenarios.push(value()?),
                "--warmup" => args.warmup = number(&value()?)?,
                "--iterations" => args.iterations = number(&value()?)?.max(1),
                "--reference" => args.reference = PathBuf::from(value()?),
                "--list" => args.list = true,
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        Ok(args)
    }
}

fn number(s: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("not a number: `{s}`"))
}

/// A fact about a scenario run, reported in `info`.
#[derive(Debug, Clone)]
pub enum Info {
    Num(f64),
    Text(String),
}

impl From<f64> for Info {
    fn from(v: f64) -> Self {
        Info::Num(v)
    }
}

impl From<usize> for Info {
    fn from(v: usize) -> Self {
        Info::Num(v as f64)
    }
}

impl From<&str> for Info {
    fn from(v: &str) -> Self {
        Info::Text(v.to_owned())
    }
}

/// What a scenario produced.
#[derive(Debug, Clone)]
pub enum Outcome {
    Ok {
        samples_ms: Vec<f64>,
        info: Vec<(&'static str, Info)>,
    },
    Skip(String),
    Error(String),
}

impl Outcome {
    pub fn ok(samples_ms: Vec<f64>) -> Self {
        Outcome::Ok {
            samples_ms,
            info: Vec::new(),
        }
    }

    /// Adds an `info` entry (no-op for skip/error).
    pub fn with(mut self, key: &'static str, value: impl Into<Info>) -> Self {
        if let Outcome::Ok { info, .. } = &mut self {
            info.push((key, value.into()));
        }
        self
    }

    /// `SKIP: <path> missing`, the repository convention for absent optional fixtures.
    pub fn missing(path: &Path) -> Self {
        Outcome::Skip(format!("SKIP: {} missing", path.display()))
    }
}

/// Milliseconds since `t`.
pub fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Runs `step(i)` for `warmup` untimed and then `iterations` timed calls; `i` keeps counting
/// across both so a step can depend on it (a scroll position). Returns the timed samples.
pub fn sample(
    warmup: usize,
    iterations: usize,
    mut step: impl FnMut(usize) -> Result<(), String>,
) -> Result<Vec<f64>, String> {
    for i in 0..warmup {
        step(i)?;
    }
    let mut samples = Vec::with_capacity(iterations);
    for i in warmup..warmup + iterations {
        let t = Instant::now();
        step(i)?;
        samples.push(ms_since(t));
    }
    Ok(samples)
}

/// A scenario: id and implementation (`Err` becomes an `error` result).
pub type Scenario = (&'static str, fn(&Args) -> Result<Outcome, String>);

/// Parses the arguments, runs the requested scenarios and prints their result lines.
/// `manifest_dir` is the example crate's `CARGO_MANIFEST_DIR` (default fixture root).
pub fn main(manifest_dir: &str, scenarios: &[Scenario]) -> ExitCode {
    let args = match Args::parse(manifest_dir) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    if args.list || args.scenarios.is_empty() {
        for (id, _) in scenarios {
            println!("{id}");
        }
        return ExitCode::SUCCESS;
    }
    for id in &args.scenarios {
        let outcome = match scenarios.iter().find(|(s, _)| s.eq_ignore_ascii_case(id)) {
            Some((_, run)) => run(&args).unwrap_or_else(Outcome::Error),
            None => Outcome::Error(format!("this example does not implement `{id}`")),
        };
        println!("{}", result_line(id, &outcome));
    }
    ExitCode::SUCCESS
}

fn result_line(id: &str, outcome: &Outcome) -> String {
    let mut out = format!("{{\"id\":{}", json_str(id));
    match outcome {
        Outcome::Ok { samples_ms, info } => {
            let samples: Vec<String> = samples_ms.iter().map(|s| json_num(*s)).collect();
            let entries: Vec<String> = info
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        Info::Num(n) => json_num(*n),
                        Info::Text(t) => json_str(t),
                    };
                    format!("{}:{v}", json_str(k))
                })
                .collect();
            let _ = write!(
                out,
                ",\"status\":\"ok\",\"samples_ms\":[{}],\"info\":{{{}}}",
                samples.join(","),
                entries.join(",")
            );
        }
        Outcome::Skip(reason) => {
            let _ = write!(out, ",\"status\":\"skip\",\"reason\":{}", json_str(reason));
        }
        Outcome::Error(reason) => {
            let _ = write!(out, ",\"status\":\"error\",\"reason\":{}", json_str(reason));
        }
    }
    out.push('}');
    out
}

fn json_num(v: f64) -> String {
    if !v.is_finite() {
        return "null".to_owned();
    }
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
