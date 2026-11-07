//! `dashcraft detect`: content-based format detection only (the input is not
//! fully read); the `doc.detect` command.

use std::io::Write;
use std::path::PathBuf;

use anyhow::anyhow;
use pbi_ops::detect_model::DetectOut;
use serde_json::json;

use crate::common::{Context, Status, print_json, with_stdout};
use crate::ops::{Ops, path_param, typed};

#[derive(Debug, clap::Args)]
pub struct DetectArgs {
    /// File or folder to identify.
    pub path: PathBuf,
    /// Print JSON (`schemaVersion` 1) instead of text.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
}

pub fn run(args: &DetectArgs, _ctx: &Context) -> anyhow::Result<Status> {
    tracing::info!(path = %args.path.display(), "detecting");
    let ops = Ops::new()?;
    let value = ops.execute("doc.detect", json!({ "path": path_param(&args.path)? }))?;
    let out: DetectOut = typed(&value)?;
    if args.json || args.pretty {
        print_json(&value, args.pretty)?;
    } else {
        with_stdout(|w| write_text(w, &out))?;
    }
    if out.detection.format == "unknown" {
        return Err(anyhow!("{}: not a recognised Power BI format", out.path));
    }
    Ok(Status::Ok)
}

fn write_text(w: &mut dyn Write, out: &DetectOut) -> std::io::Result<()> {
    let d = &out.detection;
    writeln!(
        w,
        "{}: {} ({}, confidence {})",
        out.path, d.label, out.source_format, d.confidence
    )?;
    if !d.signals.is_empty() {
        writeln!(w, "  Signals:")?;
        for s in &d.signals {
            writeln!(w, "    - {s}")?;
        }
    }
    let items = &out.items;
    if let Some(p) = &items.pbip_file {
        writeln!(w, "  Project file: {p}")?;
    }
    let shown = |p: &str| {
        if p.is_empty() {
            ".".to_owned()
        } else {
            p.to_owned()
        }
    };
    if !items.semantic_models.is_empty() {
        writeln!(w, "  Semantic models:")?;
        for m in &items.semantic_models {
            let cache = if m.has_cache_abf {
                ", .pbi/cache.abf"
            } else {
                ""
            };
            writeln!(w, "    {} ({}{cache})", shown(&m.path), m.flavour)?;
        }
    }
    if !items.reports.is_empty() {
        writeln!(w, "  Reports:")?;
        for r in &items.reports {
            let dataset = match &r.dataset_path {
                Some(p) => format!("{} {p}", r.dataset),
                None => r.dataset.clone(),
            };
            writeln!(w, "    {} ({}, {dataset})", shown(&r.path), r.flavour)?;
        }
    }
    if !items.other_items.is_empty() {
        writeln!(w, "  Other items:")?;
        for i in &items.other_items {
            writeln!(w, "    {} ({})", shown(&i.path), i.item_type)?;
        }
    }
    Ok(())
}
