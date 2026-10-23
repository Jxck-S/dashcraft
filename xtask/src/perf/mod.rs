//! `cargo xtask perf`: performance scenarios with budgets and a per-machine baseline.
//!
//! Scenarios are listed in `perf/budgets.toml`. Each one is implemented by a `perf_*` example
//! of the crate it measures. All perf examples are built by one `cargo build --release`
//! (so features unify the same way on every run) and run as
//! `<example> --scenario <id> --warmup <n> --iterations <n> --reference <dir>`; the example
//! prints one JSON line with its samples (protocol in `perf/harness.rs`). Results are compared
//! with the budgets and with this machine's entry in `perf/baseline.json`, printed, and
//! written to `target/perf/report.{json,md}`. See `docs/perf.md`.

mod baseline;
mod budgets;
mod report;
mod stats;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use self::baseline::{Baseline, Machine, ScenarioBaseline};
use self::budgets::{Budgets, Scenario};
use self::report::{Report, Run, Status, evaluate};
use crate::util;

const USAGE: &str = "\
usage: cargo xtask perf [options]

Builds the perf_* examples in release mode, runs the scenarios of perf/budgets.toml and
compares them with their budgets and with this machine's entry in perf/baseline.json.
Writes target/perf/report.json and target/perf/report.md.

options:
  --quick               fewer iterations (CI)
  --scenario <ID>       run only this scenario (id like P01, or name); repeatable, comma lists ok
  --list                list the scenarios and exit
  --warmup <N>          override the warmup count of every scenario
  --iterations <N>      override the iteration count of every scenario
  --reference <DIR>     fixture root (default: $PBIVIEW_REFERENCE, else ./reference)
  --update-baseline     record the results as this machine's baseline (regressions do not fail)
  --json                print the JSON report on stdout (the table goes to stderr)
  --no-fail             exit 0 even when a scenario fails its budget or regresses
  --locked              pass --locked to cargo build

exit status: 0 ok; 1 a scenario errored, exceeded max_ms or regressed; 2 usage or build error.
";

#[derive(Debug, Default)]
struct Options {
    quick: bool,
    scenarios: Vec<String>,
    list: bool,
    warmup: Option<usize>,
    iterations: Option<usize>,
    reference: Option<PathBuf>,
    update_baseline: bool,
    json: bool,
    no_fail: bool,
    locked: bool,
}

impl Options {
    /// `None` for `--help`.
    fn parse(args: &[String]) -> Result<Option<Self>> {
        let mut o = Options::default();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f, Some(v.to_owned())),
                _ => (arg.as_str(), None),
            };
            let mut value = || -> Result<String> {
                match inline.clone().or_else(|| it.next().cloned()) {
                    Some(v) => Ok(v),
                    None => bail!("`{flag}` needs a value"),
                }
            };
            match flag {
                "-h" | "--help" => return Ok(None),
                "--quick" => o.quick = true,
                "--list" => o.list = true,
                "--update-baseline" => o.update_baseline = true,
                "--json" => o.json = true,
                "--no-fail" => o.no_fail = true,
                "--locked" => o.locked = true,
                "--scenario" | "-s" => o.scenarios.extend(
                    value()?
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                ),
                "--warmup" => o.warmup = Some(parse_count(flag, &value()?)?),
                "--iterations" => {
                    let n = parse_count(flag, &value()?)?;
                    if n == 0 {
                        bail!("`--iterations` must be at least 1");
                    }
                    o.iterations = Some(n);
                }
                "--reference" => o.reference = Some(PathBuf::from(value()?)),
                other => bail!("unknown option `{other}` (see `cargo xtask perf --help`)"),
            }
        }
        Ok(Some(o))
    }
}

fn parse_count(flag: &str, v: &str) -> Result<usize> {
    v.parse()
        .with_context(|| format!("`{flag}` expects a number, got `{v}`"))
}

/// `cargo xtask perf ...`; `Ok(false)` when a scenario fails.
pub fn main(args: &[String]) -> Result<bool> {
    let Some(opts) = Options::parse(args)? else {
        print!("{USAGE}");
        return Ok(true);
    };
    let root = util::workspace_root();
    let budgets = Budgets::load(&root.join("perf/budgets.toml"))?;
    if opts.list {
        print_list(&budgets);
        return Ok(true);
    }
    let selected = select(&budgets, &opts.scenarios)?;
    let reference = opts
        .reference
        .clone()
        .or_else(|| std::env::var_os("PBIVIEW_REFERENCE").map(PathBuf::from))
        .unwrap_or_else(|| root.join("reference"));
    let target_dir = target_dir()?;
    let exes = build(&budgets, opts.locked)?;

    let machine = Machine::detect();
    let baseline_path = root.join("perf/baseline.json");
    let mut baseline = Baseline::load(&baseline_path)?;
    let has_baseline = baseline.machines.contains_key(&machine.key);
    let mut results = Vec::new();
    for sc in &selected {
        let (mut warmup, mut iterations) = sc.counts(&budgets.settings, opts.quick);
        warmup = opts.warmup.unwrap_or(warmup);
        iterations = opts.iterations.unwrap_or(iterations);
        eprintln!(
            "perf: {} {}: {warmup} warmup + {iterations} iteration(s){}",
            sc.id,
            sc.name,
            if sc.per_process {
                ", one process each"
            } else {
                ""
            }
        );
        let run = match exes.get(&sc.example) {
            Some(exe) => run_scenario(exe, sc, warmup, iterations, &reference),
            None => Run::Failed(format!("example `{}` was not built", sc.example)),
        };
        results.push(evaluate(
            sc,
            &budgets.settings,
            run,
            baseline.scenario(&machine.key, &sc.id),
        ));
    }

    let report = Report::new(
        machine.clone(),
        opts.quick,
        &budgets.settings,
        has_baseline,
        results,
    );
    let out_dir = target_dir.join("perf");
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("cannot create {}", out_dir.display()))?;
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(out_dir.join("report.json"), format!("{json}\n"))?;
    std::fs::write(out_dir.join("report.md"), report.markdown())?;
    if opts.json {
        println!("{json}");
        eprint!("\n{}", report.table());
    } else {
        print!("\n{}", report.table());
    }
    eprintln!("perf: report written to {}", out_dir.display());

    let mut failures = report.failures;
    if opts.update_baseline {
        let recorded: Vec<(String, ScenarioBaseline)> = report
            .results
            .iter()
            .filter_map(|r| {
                let stats = r.stats?;
                Some((
                    r.id.clone(),
                    ScenarioBaseline {
                        metric: r.metric,
                        value_ms: r.value_ms?,
                        stats,
                        quick: opts.quick,
                    },
                ))
            })
            .collect();
        let n = recorded.len();
        baseline.record(&machine, recorded);
        baseline.save(&baseline_path)?;
        eprintln!(
            "perf: recorded {n} scenario(s) for {} in {}",
            machine.key,
            baseline_path.display()
        );
        failures = report
            .results
            .iter()
            .filter(|r| r.status.fails() && r.status != Status::Regressed)
            .count();
    }
    if failures > 0 {
        eprintln!(
            "perf: {failures} scenario(s) failed{}",
            if opts.no_fail {
                " (ignored: --no-fail)"
            } else {
                ""
            }
        );
        return Ok(opts.no_fail);
    }
    Ok(true)
}

fn print_list(budgets: &Budgets) {
    for sc in &budgets.scenarios {
        println!(
            "{}  {:<24} {:<28} {} ≤ {} ms (max {} ms), {}{}",
            sc.id,
            sc.name,
            format!("{}/{}", sc.package, sc.example),
            sc.metric,
            sc.target_ms,
            sc.max_ms,
            sc.data,
            if sc.per_process { ", per process" } else { "" }
        );
        println!("     {}", sc.description.trim());
    }
}

fn select<'a>(budgets: &'a Budgets, keys: &[String]) -> Result<Vec<&'a Scenario>> {
    if keys.is_empty() {
        return Ok(budgets.scenarios.iter().collect());
    }
    for k in keys {
        if !budgets.scenarios.iter().any(|s| s.matches(k)) {
            bail!("unknown scenario `{k}` (see `cargo xtask perf --list`)");
        }
    }
    Ok(budgets
        .scenarios
        .iter()
        .filter(|s| keys.iter().any(|k| s.matches(k)))
        .collect())
}

fn target_dir() -> Result<PathBuf> {
    let meta = util::cargo_metadata(&["--no-deps"])?;
    meta["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .context("`cargo metadata` without `target_directory`")
}

/// Builds every perf example in one release build; example name → executable.
fn build(budgets: &Budgets, locked: bool) -> Result<HashMap<String, PathBuf>> {
    let examples = budgets.examples();
    let mut cmd = util::cargo();
    cmd.args([
        "build",
        "--release",
        "--message-format=json-render-diagnostics",
    ]);
    if locked {
        cmd.arg("--locked");
    }
    let mut packages: Vec<&str> = Vec::new();
    for (p, _) in &examples {
        if !packages.contains(&p.as_str()) {
            packages.push(p);
            cmd.args(["-p", p]);
        }
    }
    for (_, e) in &examples {
        cmd.args(["--example", e]);
    }
    eprintln!(
        "perf: building {} in release mode",
        examples
            .iter()
            .map(|(p, e)| format!("{p}/{e}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .context("cannot run `cargo build`")?;
    if !output.status.success() {
        bail!("building the perf examples failed ({})", output.status);
    }
    let exes = built_examples(&String::from_utf8_lossy(&output.stdout));
    for (_, e) in &examples {
        if !exes.contains_key(e) {
            bail!("`cargo build` did not report an executable for example `{e}`");
        }
    }
    Ok(exes)
}

/// Example executables from `cargo build --message-format=json` output.
fn built_examples(messages: &str) -> HashMap<String, PathBuf> {
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| {
            m["target"]["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|k| k == "example"))
        })
        .filter_map(|m| {
            Some((
                m["target"]["name"].as_str()?.to_owned(),
                PathBuf::from(m["executable"].as_str()?),
            ))
        })
        .collect()
}

fn run_scenario(
    exe: &Path,
    sc: &Scenario,
    warmup: usize,
    iterations: usize,
    reference: &Path,
) -> Run {
    if !sc.per_process {
        return run_once(exe, &sc.id, warmup, iterations, reference);
    }
    let mut samples = Vec::new();
    let mut info = Map::new();
    for i in 0..warmup + iterations {
        match run_once(exe, &sc.id, 0, 1, reference) {
            Run::Samples {
                samples: s,
                info: inf,
            } => {
                if i >= warmup {
                    samples.extend(s);
                    info = inf;
                }
            }
            other => return other,
        }
    }
    Run::Samples { samples, info }
}

fn run_once(exe: &Path, id: &str, warmup: usize, iterations: usize, reference: &Path) -> Run {
    let output = Command::new(exe)
        .args(["--scenario", id])
        .args(["--warmup", &warmup.to_string()])
        .args(["--iterations", &iterations.to_string()])
        .arg("--reference")
        .arg(reference)
        .env("PBIVIEW_REFERENCE", reference)
        .current_dir(util::workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output();
    match output {
        Ok(o) => parse_result(id, &String::from_utf8_lossy(&o.stdout), o.status),
        Err(e) => Run::Failed(format!("cannot start {}: {e}", exe.display())),
    }
}

/// The result line for `id` in an example's stdout.
fn parse_result(id: &str, stdout: &str, status: impl fmt::Display) -> Run {
    let line = stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["id"] == id);
    let Some(v) = line else {
        return Run::Failed(format!("no result line ({status})"));
    };
    let reason = || v["reason"].as_str().unwrap_or("no reason given").to_owned();
    match v["status"].as_str() {
        Some("ok") => Run::Samples {
            samples: v["samples_ms"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default(),
            info: v["info"].as_object().cloned().unwrap_or_default(),
        },
        Some("skip") => Run::Skipped(reason()),
        _ => Run::Failed(reason()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse() {
        let args: Vec<String> = [
            "--quick",
            "--scenario",
            "P01,p02",
            "--scenario=table-scroll-step",
            "--iterations=3",
            "--no-fail",
            "--json",
        ]
        .map(str::to_owned)
        .to_vec();
        let o = Options::parse(&args).unwrap().unwrap();
        assert!(o.quick && o.no_fail && o.json && !o.update_baseline);
        assert_eq!(o.scenarios, ["P01", "p02", "table-scroll-step"]);
        assert_eq!(o.iterations, Some(3));
        assert!(Options::parse(&["--help".to_owned()]).unwrap().is_none());
        assert!(Options::parse(&["--bogus".to_owned()]).is_err());
        assert!(Options::parse(&["--iterations".to_owned()]).is_err());
        assert!(Options::parse(&["--iterations=0".to_owned()]).is_err());
    }

    #[test]
    fn result_lines() {
        let status = "exit status: 0";
        let out = "progress\n{\"id\":\"P01\",\"status\":\"ok\",\"samples_ms\":[1.5,2],\
                   \"info\":{\"rows\":50000}}\n";
        assert_eq!(
            parse_result("P01", out, status),
            Run::Samples {
                samples: vec![1.5, 2.0],
                info: serde_json::json!({"rows": 50000})
                    .as_object()
                    .unwrap()
                    .clone(),
            }
        );
        let skip = "{\"id\":\"P08\",\"status\":\"skip\",\"reason\":\"SKIP: x missing\"}";
        assert_eq!(
            parse_result("P08", skip, status),
            Run::Skipped("SKIP: x missing".into())
        );
        let err = "{\"id\":\"P03\",\"status\":\"error\",\"reason\":\"boom\"}";
        assert_eq!(parse_result("P03", err, status), Run::Failed("boom".into()));
        assert!(matches!(parse_result("P04", out, status), Run::Failed(_)));
    }

    #[test]
    fn example_executables_from_cargo_messages() {
        let messages = r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"pbi_m"},"executable":null}
{"reason":"compiler-artifact","target":{"kind":["example"],"name":"perf_m"},"executable":"/t/release/examples/perf_m"}
{"reason":"build-finished","success":true}"#;
        let exes = built_examples(messages);
        assert_eq!(exes.len(), 1);
        assert_eq!(exes["perf_m"], PathBuf::from("/t/release/examples/perf_m"));
    }
}
