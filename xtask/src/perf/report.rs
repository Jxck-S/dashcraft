//! Comparing results with budgets and the baseline; table, Markdown and JSON output.

use serde::Serialize;
use serde_json::{Map, Value};

use super::baseline::{Machine, ScenarioBaseline};
use super::budgets::{DataSource, Metric, Scenario, Settings};
use super::stats::Stats;

/// What a scenario run produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    Samples {
        samples: Vec<f64>,
        info: Map<String, Value>,
    },
    Skipped(String),
    Failed(String),
}

/// Outcome of one scenario, most severe first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Error,
    OverMax,
    Regressed,
    OverTarget,
    Ok,
    Skipped,
}

impl Status {
    /// Fails the run (unless `--no-fail`).
    pub fn fails(self) -> bool {
        matches!(self, Status::Error | Status::OverMax | Status::Regressed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Error => "ERROR",
            Status::OverMax => "FAIL (over max)",
            Status::Regressed => "FAIL (regressed)",
            Status::OverTarget => "over target",
            Status::Ok => "ok",
            Status::Skipped => "skipped",
        }
    }
}

/// One scenario's evaluated result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScenarioResult {
    pub id: String,
    pub name: String,
    pub metric: Metric,
    pub target_ms: f64,
    pub max_ms: f64,
    pub data: DataSource,
    pub status: Status,
    pub value_ms: Option<f64>,
    pub stats: Option<Stats>,
    pub baseline_ms: Option<f64>,
    pub delta_pct: Option<f64>,
    pub over_target: bool,
    pub over_max: bool,
    pub regressed: bool,
    /// Why it was skipped or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Scenario facts reported by the example (sizes, sub-timings). Never data values.
    pub info: Map<String, Value>,
}

/// Compares `run` with the budgets of `scenario` and with `baseline` (same machine).
pub fn evaluate(
    scenario: &Scenario,
    settings: &Settings,
    run: Run,
    baseline: Option<&ScenarioBaseline>,
) -> ScenarioResult {
    let mut result = ScenarioResult {
        id: scenario.id.clone(),
        name: scenario.name.clone(),
        metric: scenario.metric,
        target_ms: scenario.target_ms,
        max_ms: scenario.max_ms,
        data: scenario.data,
        status: Status::Ok,
        value_ms: None,
        stats: None,
        baseline_ms: None,
        delta_pct: None,
        over_target: false,
        over_max: false,
        regressed: false,
        reason: None,
        info: Map::new(),
    };
    let baseline = baseline.filter(|b| b.metric == scenario.metric && b.value_ms > 0.0);
    result.baseline_ms = baseline.map(|b| b.value_ms);
    let (samples, info) = match run {
        Run::Samples { samples, info } => (samples, info),
        Run::Skipped(reason) => {
            result.status = Status::Skipped;
            result.reason = Some(reason);
            return result;
        }
        Run::Failed(reason) => {
            result.status = Status::Error;
            result.reason = Some(reason);
            return result;
        }
    };
    result.info = info;
    let Some(stats) = Stats::from_samples(&samples) else {
        result.status = Status::Error;
        result.reason = Some("no samples".to_owned());
        return result;
    };
    let value = stats.metric(scenario.metric);
    result.stats = Some(stats);
    result.value_ms = Some(value);
    result.over_target = value > scenario.target_ms;
    result.over_max = value > scenario.max_ms;
    if let Some(b) = baseline {
        result.delta_pct = Some(((value / b.value_ms - 1.0) * 1000.0).round() / 10.0);
        result.regressed = value > b.value_ms * (1.0 + settings.regression_pct / 100.0)
            && value - b.value_ms > settings.regression_floor_ms;
    }
    result.status = if result.over_max {
        Status::OverMax
    } else if result.regressed {
        Status::Regressed
    } else if result.over_target {
        Status::OverTarget
    } else {
        Status::Ok
    };
    result
}

/// The run as written to `target/perf/report.json` and printed by `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: u32,
    pub machine: Machine,
    pub quick: bool,
    pub regression_pct: f64,
    pub regression_floor_ms: f64,
    /// Whether `perf/baseline.json` has an entry for this machine.
    pub baseline_for_machine: bool,
    pub results: Vec<ScenarioResult>,
    /// Scenarios whose status fails the run.
    pub failures: usize,
}

impl Report {
    pub fn new(
        machine: Machine,
        quick: bool,
        settings: &Settings,
        baseline_for_machine: bool,
        results: Vec<ScenarioResult>,
    ) -> Self {
        let failures = results.iter().filter(|r| r.status.fails()).count();
        Self {
            schema: 1,
            machine,
            quick,
            regression_pct: settings.regression_pct,
            regression_floor_ms: settings.regression_floor_ms,
            baseline_for_machine,
            results,
            failures,
        }
    }

    fn header(&self) -> String {
        format!(
            "{} {} ({}, {} cores), {}; {} mode; baseline: {}",
            self.machine.os,
            self.machine.os_version,
            self.machine.cpu,
            self.machine.cores,
            self.machine.rustc,
            if self.quick { "quick" } else { "full" },
            if self.baseline_for_machine {
                format!(
                    "{} (regression: > +{}% and > {} ms)",
                    self.machine.key, self.regression_pct, self.regression_floor_ms
                )
            } else {
                format!("none for {}", self.machine.key)
            }
        )
    }

    fn rows(&self) -> Vec<[String; 9]> {
        self.results
            .iter()
            .map(|r| {
                let status = match &r.reason {
                    Some(reason) => format!("{}: {reason}", r.status.label()),
                    None => r.status.label().to_owned(),
                };
                [
                    r.id.clone(),
                    r.name.clone(),
                    r.metric.to_string(),
                    r.value_ms.map(fmt_ms).unwrap_or_else(|| "-".into()),
                    fmt_budget(r.target_ms),
                    fmt_budget(r.max_ms),
                    r.baseline_ms.map(fmt_ms).unwrap_or_else(|| "-".into()),
                    r.delta_pct
                        .map(|d| format!("{d:+.0}%"))
                        .unwrap_or_else(|| "-".into()),
                    status,
                ]
            })
            .collect()
    }

    fn summary(&self) -> String {
        let count = |s: Status| self.results.iter().filter(|r| r.status == s).count();
        format!(
            "{} scenario(s): {} ok, {} over target, {} over max, {} regressed, {} error(s), {} skipped",
            self.results.len(),
            count(Status::Ok),
            count(Status::OverTarget),
            count(Status::OverMax),
            count(Status::Regressed),
            count(Status::Error),
            count(Status::Skipped),
        )
    }

    /// Plain-text table.
    pub fn table(&self) -> String {
        const HEAD: [&str; 9] = [
            "ID", "Scenario", "Metric", "ms", "Target", "Max", "Baseline", "Δ", "Status",
        ];
        let rows = self.rows();
        let mut widths = HEAD.map(|h| h.chars().count());
        for row in &rows {
            for (w, cell) in widths.iter_mut().zip(row).take(8) {
                *w = (*w).max(cell.chars().count());
            }
        }
        let line = |cells: &[String]| {
            let mut out = String::new();
            for (i, (cell, w)) in cells.iter().zip(widths).enumerate() {
                let pad = w.saturating_sub(cell.chars().count());
                // Numbers right-aligned, text left-aligned; the last column is not padded.
                if i == 8 {
                    out.push_str(cell);
                } else if (3..=7).contains(&i) {
                    out.push_str(&" ".repeat(pad));
                    out.push_str(cell);
                    out.push_str("  ");
                } else {
                    out.push_str(cell);
                    out.push_str(&" ".repeat(pad + 2));
                }
            }
            out.trim_end().to_owned()
        };
        let mut out = format!("perf: {}\n\n", self.header());
        out.push_str(&line(&HEAD.map(str::to_owned)));
        out.push('\n');
        for row in &rows {
            out.push_str(&line(row));
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&self.summary());
        out.push('\n');
        out
    }

    /// Markdown (CI step summary).
    pub fn markdown(&self) -> String {
        let mut out = format!(
            "### Performance budgets\n\n{}\n\n\
             | ID | Scenario | Metric | ms | Target | Max | Baseline | Δ | Status |\n\
             |---|---|---|--:|--:|--:|--:|--:|---|\n",
            self.header()
        );
        for row in self.rows() {
            let cells: Vec<String> = row.iter().map(|c| c.replace('|', "\\|")).collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
        }
        out.push_str(&format!("\n{}\n", self.summary()));
        out
    }
}

/// A budget: whole milliseconds without decimals.
fn fmt_budget(ms: f64) -> String {
    if ms.fract() == 0.0 {
        format!("{ms:.0}")
    } else {
        fmt_ms(ms)
    }
}

/// Milliseconds with precision that suits the magnitude.
pub fn fmt_ms(ms: f64) -> String {
    if ms < 10.0 {
        format!("{ms:.2}")
    } else if ms < 100.0 {
        format!("{ms:.1}")
    } else {
        format!("{ms:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::budgets::Budgets;

    fn budgets() -> Budgets {
        Budgets::parse(
            r#"
            [settings]
            warmup = 1
            iterations = 5
            quick_warmup = 0
            quick_iterations = 3
            regression_pct = 20
            regression_floor_ms = 1.0

            [[scenario]]
            id = "P01"
            name = "scroll"
            description = "d"
            package = "p"
            example = "perf_x"
            target_ms = 10
            max_ms = 20
            "#,
        )
        .unwrap()
    }

    fn samples(v: &[f64]) -> Run {
        Run::Samples {
            samples: v.to_vec(),
            info: Map::new(),
        }
    }

    fn base(value_ms: f64) -> ScenarioBaseline {
        ScenarioBaseline {
            metric: Metric::Median,
            value_ms,
            stats: Stats::from_samples(&[value_ms]).unwrap(),
            quick: false,
        }
    }

    fn status(run: Run, baseline: Option<f64>) -> (Status, Option<f64>) {
        let b = budgets();
        let baseline = baseline.map(base);
        let r = evaluate(&b.scenarios[0], &b.settings, run, baseline.as_ref());
        (r.status, r.delta_pct.map(f64::round))
    }

    #[test]
    fn budgets_decide_without_a_baseline() {
        assert_eq!(status(samples(&[5.0, 9.0, 30.0]), None), (Status::Ok, None));
        assert_eq!(status(samples(&[12.0]), None), (Status::OverTarget, None));
        assert_eq!(status(samples(&[25.0]), None), (Status::OverMax, None));
        assert_eq!(status(samples(&[]), None).0, Status::Error);
        assert_eq!(status(Run::Skipped("x".into()), None).0, Status::Skipped);
        assert_eq!(status(Run::Failed("x".into()), Some(1.0)).0, Status::Error);
    }

    #[test]
    fn regressions_need_both_the_percentage_and_the_floor() {
        // +50% but only 0.5 ms: noise.
        assert_eq!(status(samples(&[1.5]), Some(1.0)), (Status::Ok, Some(50.0)));
        // +25% and 2 ms.
        assert_eq!(
            status(samples(&[10.0]), Some(8.0)),
            (Status::Regressed, Some(25.0))
        );
        // +15%: within the threshold, but over target.
        assert_eq!(
            status(samples(&[11.5]), Some(10.0)),
            (Status::OverTarget, Some(15.0))
        );
        // Over max wins over regressed.
        assert_eq!(status(samples(&[30.0]), Some(10.0)).0, Status::OverMax);
        // Faster is fine.
        assert_eq!(
            status(samples(&[4.0]), Some(8.0)),
            (Status::Ok, Some(-50.0))
        );
    }

    #[test]
    fn baselines_of_another_metric_are_ignored() {
        let b = budgets();
        let mut other = base(1.0);
        other.metric = Metric::P95;
        let r = evaluate(&b.scenarios[0], &b.settings, samples(&[9.0]), Some(&other));
        assert_eq!((r.status, r.baseline_ms), (Status::Ok, None));
    }

    #[test]
    fn failures_count_only_failing_statuses() {
        assert!(Status::Error.fails() && Status::OverMax.fails() && Status::Regressed.fails());
        assert!(!Status::OverTarget.fails() && !Status::Ok.fails() && !Status::Skipped.fails());
    }

    #[test]
    fn number_formatting() {
        assert_eq!(fmt_ms(1.234), "1.23");
        assert_eq!(fmt_ms(12.34), "12.3");
        assert_eq!(fmt_ms(1234.4), "1234");
        assert_eq!(
            (fmt_budget(16.0), fmt_budget(2.5)),
            ("16".into(), "2.50".into())
        );
    }
}
