//! `dashcraft convert`: writes an input as a PBIP project (TMDL + PBIR) or a
//! PBIT template.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use pbi_core::{OpenedDocument, SaveOptions, export_pbit, save_as_pbip};
use pbi_model::{Severity, names_eq};

use crate::common::{Context, ReadArgs, Status, open_document};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Target {
    /// PBIP project folder: `.pbip`, `*.SemanticModel` (TMDL) and
    /// `*.Report` (PBIR) folders.
    Pbip,
    /// PBIT template file: model schema and one report, no data.
    Pbit,
}

#[derive(Debug, clap::Args)]
pub struct ConvertArgs {
    /// File or folder to convert.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum)]
    pub to: Target,
    /// Output folder (--to pbip) or file (--to pbit).
    #[arg(long, short)]
    pub out: PathBuf,
    /// Write into a non-empty output folder (files with the same paths are
    /// replaced) or overwrite an existing output file.
    #[arg(long)]
    pub force: bool,
    /// --to pbit: name of the report to include (a PBIT holds one report;
    /// default: the first).
    #[arg(long, value_name = "NAME")]
    pub report: Option<String>,
    #[command(flatten)]
    pub read: ReadArgs,
}

pub fn run(args: &ConvertArgs, ctx: &Context) -> anyhow::Result<Status> {
    if args.report.is_some() && args.to != Target::Pbit {
        bail!("--report only applies to --to pbit");
    }
    check_output(&args.out, args.to, args.force)?;
    let mut opened = open_document(&args.path, &args.read.open_options())?;
    let out = &args.out;
    match args.to {
        Target::Pbip => {
            save_as_pbip(
                &opened,
                out,
                &SaveOptions {
                    overwrite: args.force,
                },
            )
            .map_err(|e| anyhow!(e).context(format!("cannot write {}", out.display())))?;
            ctx.note(format!("wrote PBIP project to {}", out.display()));
        }
        Target::Pbit => {
            select_report(&mut opened, args.report.as_deref(), ctx)?;
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            export_pbit(&opened, out)
                .map_err(|e| anyhow!(e).context(format!("cannot write {}", out.display())))?;
            ctx.note(format!("wrote PBIT to {}", out.display()));
        }
    }
    for d in &opened.document.diagnostics {
        let origin = d
            .origin
            .as_ref()
            .map(|o| format!(" ({o})"))
            .unwrap_or_default();
        match d.severity {
            Severity::Error => ctx.note(format!("error: {} {}{origin}", d.code, d.message)),
            Severity::Warning => ctx.warn(format!("{} {}{origin}", d.code, d.message)),
            Severity::Info => {}
        }
    }
    Ok(Status::of(&opened.document))
}

/// Refuses to replace existing, non-empty outputs unless `force`.
fn check_output(out: &Path, to: Target, force: bool) -> anyhow::Result<()> {
    let Ok(meta) = out.metadata() else {
        return Ok(());
    };
    match to {
        Target::Pbip => {
            if !meta.is_dir() {
                bail!("{} exists and is not a folder", out.display());
            }
            let mut entries =
                fs::read_dir(out).with_context(|| format!("cannot read {}", out.display()))?;
            if entries.next().is_some() && !force {
                bail!(
                    "{} is not empty; use --force to write into it",
                    out.display()
                );
            }
        }
        Target::Pbit => {
            if meta.is_dir() {
                bail!("{} is a folder; --to pbit writes a file", out.display());
            }
            if meta.len() > 0 && !force {
                bail!(
                    "{} already exists; use --force to overwrite it",
                    out.display()
                );
            }
        }
    }
    Ok(())
}

/// Moves the requested report first: a PBIT holds the first report only.
fn select_report(
    opened: &mut OpenedDocument,
    name: Option<&str>,
    ctx: &Context,
) -> anyhow::Result<()> {
    let reports = &opened.document.reports;
    if reports.len() != opened.report_sources.len() {
        bail!("internal error: reports and report sources are not aligned");
    }
    let names = || {
        reports
            .iter()
            .map(|r| format!("'{}'", r.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let index = match name {
        Some(name) => reports
            .iter()
            .position(|r| names_eq(&r.name, name))
            .ok_or_else(|| anyhow!("no report named '{name}'; reports: {}", names()))?,
        None => {
            if reports.len() > 1 {
                ctx.warn(format!(
                    "a PBIT holds one report; writing '{}' (choose with --report; reports: {})",
                    reports[0].name,
                    names()
                ));
            }
            0
        }
    };
    if index > 0 {
        let report = opened.document.reports.remove(index);
        opened.document.reports.insert(0, report);
        let source = opened.report_sources.remove(index);
        opened.report_sources.insert(0, source);
    }
    Ok(())
}
