//! `cargo xtask groundtruth`: coverage of the Desktop ground truth.
//!
//! Reads `reference/groundtruth/{corpus,recordings}/*.json` and
//! `deviations.json` (JSON only, no engine) and prints per suite: cases,
//! recorded cases, cases whose query changed since recording (re-record),
//! volatile (shape-only) cases, ledger entries, and the oracle's version (Power BI Desktop, or
//! Microsoft's TOM library for the TMDL suites).
//! The comparison itself is `cargo test -p pbi-session --test groundtruth` (DAX suites),
//! `cargo test -p pbi-m --test groundtruth` (M suites) and
//! `cargo test -p pbi-tmdl --test groundtruth` (TMDL suites).
//! With `--check`, a missing recording, an unrecorded case or a changed query
//! fails. See `reference/groundtruth/README.md`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::util::workspace_root;

const USAGE: &str = "\
usage: cargo xtask groundtruth [--check]

Prints the coverage of reference/groundtruth (cases, recorded, query changed,
volatile, ledger entries, oracle version) per suite. --check fails when a
corpus has no recording, a case is not recorded or its query changed.
";

#[derive(Debug, Default, PartialEq)]
struct SuiteStatus {
    suite: String,
    cases: usize,
    recorded: usize,
    changed: usize,
    volatile: usize,
    ledger: usize,
    /// Version of the oracle: `2.159…` (Desktop) or `TOM 19.118.3`.
    oracle: Option<String>,
    has_recording: bool,
    tmdl: bool,
}

impl SuiteStatus {
    fn is_complete(&self) -> bool {
        self.has_recording && self.recorded == self.cases && self.changed == 0
    }
}

pub fn main(args: &[String]) -> Result<bool> {
    let mut check = false;
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(true);
            }
            other => anyhow::bail!("unknown option `{other}`\n\n{USAGE}"),
        }
    }
    let root = workspace_root().join("reference").join("groundtruth");
    let statuses = collect(&root)?;
    print_table(&statuses);
    let incomplete: Vec<&SuiteStatus> = statuses.iter().filter(|s| !s.is_complete()).collect();
    if !incomplete.is_empty() {
        let list = |tmdl: bool| {
            incomplete
                .iter()
                .filter(|s| s.tmdl == tmdl)
                .map(|s| s.suite.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let desktop = list(false);
        if !desktop.is_empty() {
            println!(
                "\nto record: {desktop} (scripts/groundtruth/Record-GroundTruth.ps1, Windows + Power BI Desktop)"
            );
        }
        let tom = list(true);
        if !tom.is_empty() {
            println!("\nto record: {tom} (scripts/groundtruth/Record-Tmdl.ps1, PowerShell 7)");
        }
    }
    println!(
        "compare: cargo test -p pbi-session --test groundtruth; cargo test -p pbi-m --test groundtruth; cargo test -p pbi-tmdl --test groundtruth"
    );
    Ok(!check || incomplete.is_empty())
}

fn read_json(path: &Path) -> Result<Value> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))
}

fn cases(doc: &Value) -> &[Value] {
    doc["cases"].as_array().map_or(&[], Vec::as_slice)
}

fn collect(root: &Path) -> Result<Vec<SuiteStatus>> {
    let corpus_dir = root.join("corpus");
    let mut ledger: HashMap<String, usize> = HashMap::new();
    let ledger_path = root.join("deviations.json");
    if ledger_path.exists() {
        for d in read_json(&ledger_path)?["deviations"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
        {
            if let Some(s) = d["suite"].as_str() {
                *ledger.entry(s.to_owned()).or_default() += 1;
            }
        }
    }
    let mut files: Vec<_> = fs::read_dir(&corpus_dir)
        .with_context(|| format!("cannot list {}", corpus_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let corpus = read_json(&path)?;
        let suite = corpus["suite"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let recording_path = root.join("recordings").join(format!("{suite}.json"));
        let recording = if recording_path.exists() {
            Some(read_json(&recording_path)?)
        } else {
            None
        };
        out.push(status(
            &suite,
            &corpus,
            recording.as_ref(),
            ledger.get(&suite).copied(),
        ));
    }
    Ok(out)
}

fn status(
    suite: &str,
    corpus: &Value,
    recording: Option<&Value>,
    ledger: Option<usize>,
) -> SuiteStatus {
    let recorded: BTreeMap<&str, &Value> = recording
        .map(|r| {
            cases(r)
                .iter()
                .filter_map(|c| Some((c["id"].as_str()?, c)))
                .collect()
        })
        .unwrap_or_default();
    let mut s = SuiteStatus {
        suite: suite.to_owned(),
        ledger: ledger.unwrap_or(0),
        has_recording: recording.is_some(),
        oracle: recording.and_then(|r| {
            r["desktop"]["version"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| r["oracle"]["version"].as_str().map(|v| format!("TOM {v}")))
        }),
        tmdl: corpus["language"] == "tmdl",
        ..SuiteStatus::default()
    };
    for case in cases(corpus) {
        s.cases += 1;
        if case.get("volatile").is_some() {
            s.volatile += 1;
        }
        match case["id"].as_str().and_then(|id| recorded.get(id)) {
            Some(rec) if rec["query"] == case["query"] => s.recorded += 1,
            Some(_) => s.changed += 1,
            None => {}
        }
    }
    s
}

fn print_table(statuses: &[SuiteStatus]) {
    println!(
        "{:<32} {:>6} {:>9} {:>8} {:>9} {:>7}  oracle",
        "suite", "cases", "recorded", "changed", "volatile", "ledger"
    );
    for s in statuses {
        println!(
            "{:<32} {:>6} {:>9} {:>8} {:>9} {:>7}  {}",
            s.suite,
            s.cases,
            s.recorded,
            s.changed,
            s.volatile,
            s.ledger,
            s.oracle
                .as_deref()
                .unwrap_or(if s.has_recording { "?" } else { "not recorded" })
        );
    }
    let total = |f: fn(&SuiteStatus) -> usize| statuses.iter().map(f).sum::<usize>();
    println!(
        "{:<32} {:>6} {:>9} {:>8} {:>9} {:>7}",
        "total",
        total(|s| s.cases),
        total(|s| s.recorded),
        total(|s| s.changed),
        total(|s| s.volatile),
        total(|s| s.ledger)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_recorded_changed_and_volatile_cases() {
        let corpus = json!({"cases": [
            {"id": "a", "query": "EVALUATE {1}"},
            {"id": "b", "query": "EVALUATE {2}", "volatile": "clock"},
            {"id": "c", "query": "EVALUATE {3}"},
        ]});
        let recording = json!({"desktop": {"version": "2.159"}, "cases": [
            {"id": "a", "query": "EVALUATE {1}"},
            {"id": "b", "query": "EVALUATE {20}"},
        ]});
        let s = status("x", &corpus, Some(&recording), Some(2));
        assert_eq!(
            (s.cases, s.recorded, s.changed, s.volatile, s.ledger),
            (3, 1, 1, 1, 2)
        );
        assert_eq!(s.oracle.as_deref(), Some("2.159"));
        assert!(!s.is_complete());
        let s = status("x", &corpus, None, None);
        assert!(!s.has_recording && !s.is_complete());
        let corpus = json!({"language": "tmdl", "cases": [{"id": "a", "query": "q"}]});
        let recording =
            json!({"oracle": {"version": "19.118.3"}, "cases": [{"id": "a", "query": "q"}]});
        let s = status("t", &corpus, Some(&recording), None);
        assert_eq!(s.oracle.as_deref(), Some("TOM 19.118.3"));
        assert!(s.tmdl && s.is_complete());
    }
}
