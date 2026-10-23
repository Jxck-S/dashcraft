//! `perf/budgets.toml`: the scenarios and their budgets.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// The whole file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub settings: Settings,
    #[serde(rename = "scenario")]
    pub scenarios: Vec<Scenario>,
}

/// Defaults shared by every scenario.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Untimed iterations before measuring.
    pub warmup: usize,
    /// Timed iterations.
    pub iterations: usize,
    /// `--quick` overrides.
    pub quick_warmup: usize,
    pub quick_iterations: usize,
    /// A result slower than the baseline by more than this percentage regresses...
    pub regression_pct: f64,
    /// ...and by more than this many milliseconds (absolute noise floor).
    pub regression_floor_ms: f64,
}

/// Which statistic is compared with the budget and the baseline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    #[default]
    Median,
    P95,
}

impl fmt::Display for Metric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Metric::Median => "median",
            Metric::P95 => "p95",
        })
    }
}

/// Where a scenario's input comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataSource {
    /// Generated in code: always runs.
    #[default]
    Synthetic,
    /// A committed fixture: runs on a fresh clone.
    Committed,
    /// Optional gitignored data (e.g. the fetched samples): skipped when missing.
    Private,
}

impl fmt::Display for DataSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DataSource::Synthetic => "synthetic",
            DataSource::Committed => "committed",
            DataSource::Private => "private",
        })
    }
}

/// One scenario.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// `P01`, `P02`, ...
    pub id: String,
    /// Short kebab-case name.
    pub name: String,
    pub description: String,
    /// Package and example that implement it (`cargo run --release -p <package> --example
    /// <example> -- --scenario <id>`).
    pub package: String,
    pub example: String,
    #[serde(default)]
    pub metric: Metric,
    /// Goal; a slower result is reported but does not fail the run.
    pub target_ms: f64,
    /// Ceiling; a slower result fails the run.
    pub max_ms: f64,
    #[serde(default)]
    pub data: DataSource,
    /// Run one process per sample (cold-start scenarios: process-wide caches start empty).
    #[serde(default)]
    pub per_process: bool,
    pub warmup: Option<usize>,
    pub iterations: Option<usize>,
    pub quick_warmup: Option<usize>,
    pub quick_iterations: Option<usize>,
}

impl Scenario {
    /// `(warmup, iterations)` for this run.
    pub fn counts(&self, settings: &Settings, quick: bool) -> (usize, usize) {
        if quick {
            (
                self.quick_warmup.unwrap_or(settings.quick_warmup),
                self.quick_iterations.unwrap_or(settings.quick_iterations),
            )
        } else {
            (
                self.warmup.unwrap_or(settings.warmup),
                self.iterations.unwrap_or(settings.iterations),
            )
        }
    }

    /// Matches `--scenario` by id (case-insensitive) or name.
    pub fn matches(&self, key: &str) -> bool {
        self.id.eq_ignore_ascii_case(key) || self.name == key
    }
}

impl Budgets {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let budgets: Budgets = toml::from_str(text)?;
        budgets.validate()?;
        Ok(budgets)
    }

    fn validate(&self) -> Result<()> {
        let s = &self.settings;
        if s.iterations == 0 || s.quick_iterations == 0 {
            bail!("settings: iterations must be at least 1");
        }
        if !(s.regression_pct > 0.0 && s.regression_floor_ms >= 0.0) {
            bail!("settings: regression_pct must be > 0 and regression_floor_ms >= 0");
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut examples: Vec<(&str, &str)> = Vec::new();
        for sc in &self.scenarios {
            let id = sc.id.as_str();
            let digits = id.strip_prefix('P').unwrap_or_default();
            if digits.len() != 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                bail!("scenario `{id}`: id must look like P01");
            }
            if !ids.insert(id) {
                bail!("scenario `{id}` is listed twice");
            }
            if !names.insert(sc.name.as_str()) {
                bail!("scenario `{id}`: name `{}` is used twice", sc.name);
            }
            if !(sc.target_ms > 0.0 && sc.target_ms <= sc.max_ms) {
                bail!("scenario `{id}`: need 0 < target_ms <= max_ms");
            }
            if sc.package.is_empty() || !sc.example.starts_with("perf_") {
                bail!("scenario `{id}`: needs a package and a `perf_*` example");
            }
            if sc.iterations == Some(0) || sc.quick_iterations == Some(0) {
                bail!("scenario `{id}`: iterations must be at least 1");
            }
            match examples.iter().find(|(_, e)| *e == sc.example) {
                Some((p, _)) if *p != sc.package => bail!(
                    "scenario `{id}`: example `{}` is in both `{p}` and `{}`",
                    sc.example,
                    sc.package
                ),
                Some(_) => {}
                None => examples.push((&sc.package, &sc.example)),
            }
        }
        Ok(())
    }

    /// Distinct `(package, example)` pairs, in file order.
    pub fn examples(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for sc in &self.scenarios {
            let pair = (sc.package.clone(), sc.example.clone());
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &str = r#"
        [settings]
        warmup = 2
        iterations = 10
        quick_warmup = 1
        quick_iterations = 3
        regression_pct = 20
        regression_floor_ms = 0.5

        [[scenario]]
        id = "P01"
        name = "scroll"
        description = "d"
        package = "pbi-render"
        example = "perf_render"
        metric = "p95"
        target_ms = 16
        max_ms = 33
        quick_iterations = 7

        [[scenario]]
        id = "P02"
        name = "cold"
        description = "d"
        package = "pbi-render"
        example = "perf_render"
        target_ms = 100
        max_ms = 100
        per_process = true
        data = "private"
    "#;

    #[test]
    fn parses_defaults_and_overrides() {
        let b = Budgets::parse(MINI).unwrap();
        let (p01, p02) = (&b.scenarios[0], &b.scenarios[1]);
        assert_eq!(p01.metric, Metric::P95);
        assert_eq!(p01.counts(&b.settings, false), (2, 10));
        assert_eq!(p01.counts(&b.settings, true), (1, 7));
        assert_eq!(p02.metric, Metric::Median);
        assert_eq!(p02.data, DataSource::Private);
        assert!(p02.per_process && !p01.per_process);
        assert!(p01.matches("p01") && p01.matches("scroll") && !p01.matches("cold"));
        assert_eq!(
            b.examples(),
            vec![("pbi-render".to_owned(), "perf_render".to_owned())]
        );
    }

    #[test]
    fn rejects_inconsistent_scenarios() {
        let bad = [
            ("id = \"P02\"", "id = \"P01\"", "listed twice"),
            ("id = \"P02\"", "id = \"Q2\"", "look like P01"),
            ("max_ms = 33", "max_ms = 3", "target_ms"),
            ("name = \"cold\"", "name = \"scroll\"", "used twice"),
            ("per_process = true", "bogus = 1", "unknown field"),
            (
                "example = \"perf_render\"",
                "example = \"render\"",
                "perf_*",
            ),
        ];
        for (from, to, needle) in bad {
            let text = MINI.replacen(from, to, 1);
            let err = format!("{:#}", Budgets::parse(&text).unwrap_err());
            assert!(err.contains(needle), "{from} → {err}");
        }
    }

    #[test]
    fn committed_budgets_are_valid() {
        let path = crate::util::workspace_root().join("perf/budgets.toml");
        let b = Budgets::load(&path).unwrap();
        assert!(b.scenarios.len() >= 7);
        for sc in &b.scenarios {
            assert!(!sc.description.trim().is_empty(), "{}", sc.id);
        }
    }
}
