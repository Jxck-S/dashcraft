//! `dashcraft lint`: the broken-reference linter (rules in
//! `crates/pbi-core/src/lint/RULES.md`).

use std::io::Write;
use std::path::PathBuf;

use pbi_core::lint::{Finding, LintOptions, LintReport, lint, severity_rank};
use pbi_model::{Diagnostic, Severity};
use serde_json::{Value, json};

use crate::common::{Context, ReadArgs, Status, open_document, print_json, with_stdout};
use crate::inspect_model::SCHEMA_VERSION;

/// Lowest severity that makes `lint` exit with code 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Deny {
    #[default]
    Errors,
    Warnings,
    Infos,
}

impl Deny {
    fn severity(self) -> Severity {
        match self {
            Deny::Errors => Severity::Error,
            Deny::Warnings => Severity::Warning,
            Deny::Infos => Severity::Info,
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct LintArgs {
    /// File or folder to check (PBIP, report or model folder, PBIX, …).
    pub path: PathBuf,
    /// Print JSON (`schemaVersion` 1) instead of text.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
    /// Exit with code 2 on findings of this severity or worse.
    #[arg(long, value_enum, default_value_t = Deny::Errors)]
    pub deny: Deny,
    /// Skip a rule (repeatable), e.g. `--disable LINT016`.
    #[arg(long = "disable", value_name = "RULE")]
    pub disabled: Vec<String>,
    /// Skip the layout rules (LINT015 bounds, LINT016 overlaps).
    #[arg(long)]
    pub no_layout: bool,
    #[command(flatten)]
    pub read: ReadArgs,
}

pub fn run(args: &LintArgs, ctx: &Context) -> anyhow::Result<Status> {
    let doc = open_document(&args.path, &args.read.open_options())?;
    let opts = LintOptions {
        disabled_rules: args.disabled.clone(),
        layout: !args.no_layout,
        ..LintOptions::default()
    };
    let report = lint(&doc, &opts);
    // Reader errors (unloadable model, broken report) hide what the linter
    // could not check, so they fail the run too.
    let reader_errors: Vec<&Diagnostic> = doc
        .document
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    let shown = args.path.to_string_lossy().into_owned();
    if args.json || args.pretty {
        print_json(&json_out(&shown, &report, &reader_errors), args.pretty)?;
    } else {
        with_stdout(|w| write_text(w, &shown, &report, &reader_errors))?;
        for note in &report.summary.notes {
            ctx.note(note);
        }
    }
    let failed = report.fails(args.deny.severity()) || !reader_errors.is_empty();
    Ok(if failed {
        Status::ErrorDiagnostics
    } else {
        Status::Ok
    })
}

fn json_out(path: &str, report: &LintReport, reader_errors: &[&Diagnostic]) -> Value {
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "path": path,
        "summary": report.summary,
        "findings": report.findings,
        "readerErrors": reader_errors,
    })
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn location(f: &Finding) -> String {
    let Some(o) = &f.origin else {
        return String::new();
    };
    let mut out = String::new();
    if let Some(line) = o.line {
        out.push_str(&format!(":{line}"));
    }
    if let Some(pointer) = &o.pointer {
        out.push_str(&format!("#{pointer}"));
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {one}s")
    }
}

fn write_text(
    w: &mut dyn Write,
    path: &str,
    report: &LintReport,
    reader_errors: &[&Diagnostic],
) -> std::io::Result<()> {
    for (file, findings) in report.by_file() {
        writeln!(w, "{}", if file.is_empty() { "(no file)" } else { file })?;
        let mut sorted = findings;
        sorted.sort_by_key(|f| std::cmp::Reverse(severity_rank(f.severity)));
        for f in sorted {
            writeln!(
                w,
                "  {} {} {}{}",
                severity_label(f.severity),
                f.rule,
                location(f),
                f.message
            )?;
            if let Some(s) = &f.suggestion {
                writeln!(w, "      hint: {s}")?;
            }
        }
    }
    if !reader_errors.is_empty() {
        writeln!(w, "reader errors (some checks could not run):")?;
        for d in reader_errors {
            let at = d
                .origin
                .as_ref()
                .map(|o| format!(" ({o})"))
                .unwrap_or_default();
            writeln!(w, "  error {} {}{at}", d.code, d.message)?;
        }
    }
    let s = &report.summary;
    if report.findings.is_empty() && reader_errors.is_empty() {
        writeln!(w, "{path}: no problems found")
    } else {
        writeln!(
            w,
            "{path}: {}, {}, {}",
            plural(s.errors, "error"),
            plural(s.warnings, "warning"),
            plural(s.infos, "info")
        )
    }
}
