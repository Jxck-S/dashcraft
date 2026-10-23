//! `perf/baseline.json`: reference timings per machine, and machine identification.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::budgets::Metric;
use super::stats::Stats;
use crate::util::command_stdout;

pub const SCHEMA: u32 = 1;

/// The whole file. Keys are sorted so updates diff cleanly.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub schema: u32,
    #[serde(default)]
    pub about: String,
    /// By [`Machine::key`].
    #[serde(default)]
    pub machines: BTreeMap<String, MachineBaseline>,
}

/// Recorded results of one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineBaseline {
    pub machine: Machine,
    /// UTC date of the last update (`YYYY-MM-DD`).
    pub recorded: String,
    /// By scenario id.
    pub scenarios: BTreeMap<String, ScenarioBaseline>,
}

/// One scenario's recorded result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioBaseline {
    pub metric: Metric,
    /// The metric's value, compared with new runs.
    pub value_ms: f64,
    pub stats: Stats,
    /// `true` when recorded with `--quick` (fewer samples).
    #[serde(default)]
    pub quick: bool,
}

/// What the numbers were measured on. No host or user names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub key: String,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu: String,
    pub cores: usize,
    pub rustc: String,
}

impl Baseline {
    /// The file, or an empty baseline when it does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("invalid {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn empty() -> Self {
        Self {
            schema: SCHEMA,
            about: "Reference timings for `cargo xtask perf`, per machine (see docs/perf.md). \
                    Numbers only: no data values. Update with `cargo xtask perf --update-baseline`."
                .to_owned(),
            machines: BTreeMap::new(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
    }

    pub fn scenario(&self, machine: &str, id: &str) -> Option<&ScenarioBaseline> {
        self.machines.get(machine)?.scenarios.get(id)
    }

    /// Records `results` for `machine`, keeping its other scenarios.
    pub fn record(
        &mut self,
        machine: &Machine,
        results: impl IntoIterator<Item = (String, ScenarioBaseline)>,
    ) {
        let entry = self
            .machines
            .entry(machine.key.clone())
            .or_insert_with(|| MachineBaseline {
                machine: machine.clone(),
                recorded: String::new(),
                scenarios: BTreeMap::new(),
            });
        entry.machine = machine.clone();
        entry.recorded = utc_date(SystemTime::now());
        entry.scenarios.extend(results);
        self.schema = SCHEMA;
    }
}

impl Machine {
    /// The machine running this process.
    pub fn detect() -> Self {
        let os = std::env::consts::OS.to_owned();
        let arch = std::env::consts::ARCH.to_owned();
        let cpu = cpu_name().unwrap_or_else(|| "unknown-cpu".to_owned());
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        Self {
            key: machine_key(&os, &arch, &cpu),
            os_version: os_version().unwrap_or_default(),
            os,
            arch,
            cpu,
            cores,
            rustc: rustc_version().unwrap_or_default(),
        }
    }
}

/// `macos-aarch64-apple-m4-pro`: OS, architecture and CPU model, lowercase, `-`-separated.
pub fn machine_key(os: &str, arch: &str, cpu: &str) -> String {
    let raw = format!("{os} {arch} {cpu}").to_lowercase();
    let mut key = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            key.push(c);
        } else if !key.ends_with('-') {
            key.push('-');
        }
    }
    key.trim_matches('-').to_owned()
}

fn cpu_name() -> Option<String> {
    match std::env::consts::OS {
        "macos" => command_stdout("sysctl", &["-n", "machdep.cpu.brand_string"]),
        "linux" => {
            let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
            info.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_owned())
        }
        "windows" => std::env::var("PROCESSOR_IDENTIFIER").ok(),
        _ => None,
    }
}

fn os_version() -> Option<String> {
    match std::env::consts::OS {
        "macos" => command_stdout("sw_vers", &["-productVersion"]),
        "linux" => {
            let release = std::fs::read_to_string("/etc/os-release").ok()?;
            release
                .lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        }
        _ => None,
    }
}

fn rustc_version() -> Option<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    command_stdout(&rustc, &["-V"])
}

/// `YYYY-MM-DD` (UTC) of `t`.
pub fn utc_date(t: SystemTime) -> String {
    let days = t
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    let (y, m, d) = civil_from_days(i64::try_from(days).unwrap_or(0));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn machine_keys_are_slugs() {
        assert_eq!(
            machine_key("macos", "aarch64", "Apple M4 Pro"),
            "macos-aarch64-apple-m4-pro"
        );
        assert_eq!(
            machine_key("linux", "x86_64", "AMD EPYC 7763 64-Core Processor  "),
            "linux-x86-64-amd-epyc-7763-64-core-processor"
        );
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc_date(UNIX_EPOCH), "1970-01-01");
        let day = |n: u64| UNIX_EPOCH + Duration::from_secs(n * 86_400 + 3_600);
        assert_eq!(utc_date(day(11_016)), "2000-02-29");
        assert_eq!(utc_date(day(20_748)), "2026-10-22");
    }

    #[test]
    fn record_keeps_other_scenarios_and_round_trips() {
        let machine = Machine {
            key: "k".into(),
            os: "macos".into(),
            os_version: "26".into(),
            arch: "aarch64".into(),
            cpu: "cpu".into(),
            cores: 8,
            rustc: "rustc 1.99.1".into(),
        };
        let stats = Stats::from_samples(&[1.0, 2.0, 3.0]).unwrap();
        let entry = |v: f64| ScenarioBaseline {
            metric: Metric::Median,
            value_ms: v,
            stats,
            quick: false,
        };
        let mut b = Baseline::empty();
        b.record(
            &machine,
            [
                ("P01".to_owned(), entry(2.0)),
                ("P02".to_owned(), entry(5.0)),
            ],
        );
        b.record(&machine, [("P01".to_owned(), entry(1.5))]);
        assert_eq!(b.scenario("k", "P01").map(|s| s.value_ms), Some(1.5));
        assert_eq!(b.scenario("k", "P02").map(|s| s.value_ms), Some(5.0));
        assert_eq!(b.scenario("other", "P01"), None);
        let text = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Baseline>(&text).unwrap(), b);
    }

    #[test]
    fn committed_baseline_parses() {
        let path = crate::util::workspace_root().join("perf/baseline.json");
        let b = Baseline::load(&path).unwrap();
        assert_eq!(b.schema, SCHEMA);
        for (key, m) in &b.machines {
            assert_eq!(key, &m.machine.key);
        }
    }
}
