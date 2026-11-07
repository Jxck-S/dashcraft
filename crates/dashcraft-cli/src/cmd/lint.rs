//! `dashcraft lint`: the broken-reference linter (rules in
//! `crates/pbi-core/src/lint/RULES.md`); the `doc.open` and `doc.lint`
//! commands.

use std::io::Write;
use std::path::PathBuf;

use pbi_ops::commands::lint::{FindingOut, LintOut};
use pbi_ops::inspect_model::OriginOut;
use serde_json::{Value, json};

use crate::common::{Context, ReadArgs, Status, print_json, with_stdout};
use crate::ops::{Ops, typed};

/// Lowest severity that makes `lint` exit with code 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Deny {
    #[default]
    Errors,
    Warnings,
    Infos,
}

impl Deny {
    /// The `doc.lint` `deny` parameter.
    fn param(self) -> &'static str {
        match self {
            Deny::Errors => "error",
            Deny::Warnings => "warning",
            Deny::Infos => "info",
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
    let ops = Ops::new()?;
    let doc = ops.open(&args.path, &args.read)?;
    let mut value = ops.execute(
        "doc.lint",
        json!({
            "docId": doc.doc_id,
            "disable": args.disabled,
            "layout": !args.no_layout,
            "deny": args.deny.param(),
        }),
    )?;
    // Reader errors (unloadable model, broken report) hide what the linter
    // could not check, so they fail the run too (`ok` covers both).
    let out: LintOut = typed(&value)?;
    if args.json || args.pretty {
        // The `lint --json` schema has no `ok`: the exit status says it.
        if let Value::Object(map) = &mut value {
            map.shift_remove("ok");
        }
        print_json(&value, args.pretty)?;
    } else {
        with_stdout(|w| write_text(w, &out))?;
        for note in &out.summary.notes {
            ctx.note(note);
        }
    }
    Ok(if out.ok {
        Status::Ok
    } else {
        Status::ErrorDiagnostics
    })
}

/// Higher for worse severities.
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "error" => 2,
        "warning" => 1,
        _ => 0,
    }
}

/// `path:line#pointer`, as the reader diagnostics' origins print.
fn origin_text(o: &OriginOut) -> String {
    let mut out = o.path.clone();
    if let Some(line) = o.line {
        out.push_str(&format!(":{line}"));
    }
    if let Some(pointer) = &o.pointer {
        out.push_str(&format!("#{pointer}"));
    }
    out
}

fn location(f: &FindingOut) -> String {
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

/// Consecutive findings of the same file (they are sorted by file).
fn by_file(findings: &[FindingOut]) -> Vec<(&str, Vec<&FindingOut>)> {
    let mut out: Vec<(&str, Vec<&FindingOut>)> = Vec::new();
    for f in findings {
        let path = f.origin.as_ref().map_or("", |o| o.path.as_str());
        match out.last_mut() {
            Some((p, list)) if *p == path => list.push(f),
            _ => out.push((path, vec![f])),
        }
    }
    out
}

fn write_text(w: &mut dyn Write, out: &LintOut) -> std::io::Result<()> {
    for (file, findings) in by_file(&out.findings) {
        writeln!(w, "{}", if file.is_empty() { "(no file)" } else { file })?;
        let mut sorted = findings;
        sorted.sort_by_key(|f| std::cmp::Reverse(severity_rank(&f.severity)));
        for f in sorted {
            writeln!(
                w,
                "  {} {} {}{}",
                f.severity,
                f.rule,
                location(f),
                f.message
            )?;
            if let Some(s) = &f.suggestion {
                writeln!(w, "      hint: {s}")?;
            }
        }
    }
    if !out.reader_errors.is_empty() {
        writeln!(w, "reader errors (some checks could not run):")?;
        for d in &out.reader_errors {
            let at = d
                .origin
                .as_ref()
                .map(|o| format!(" ({})", origin_text(o)))
                .unwrap_or_default();
            writeln!(w, "  error {} {}{at}", d.code, d.message)?;
        }
    }
    let s = &out.summary;
    if out.findings.is_empty() && out.reader_errors.is_empty() {
        writeln!(w, "{}: no problems found", out.path)
    } else {
        writeln!(
            w,
            "{}: {}, {}, {}",
            out.path,
            plural(s.errors, "error"),
            plural(s.warnings, "warning"),
            plural(s.infos, "info")
        )
    }
}
