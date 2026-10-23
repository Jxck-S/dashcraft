//! `dashcraft detect`: content-based format detection only (the input is not
//! fully read).

use std::io::Write;
use std::path::PathBuf;

use anyhow::anyhow;
use pbi_core::container::Limits;
use pbi_core::detect::{self, DetectedFormat};
use pbi_ops::detect_model::{DetectOut, detect_out};

use crate::common::{Context, Status, open_error, print_json, with_stdout};

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
    let detection = detect::detect_path(&args.path, &Limits::default())
        .map_err(|e| open_error(&args.path, e.into()))?;
    let shown = args.path.to_string_lossy().into_owned();
    let out = detect_out(&shown, &detection);
    if args.json || args.pretty {
        print_json(&out, args.pretty)?;
    } else {
        with_stdout(|w| write_text(w, &out))?;
    }
    if detection.format == DetectedFormat::Unknown {
        return Err(anyhow!("{shown}: not a recognised Power BI format"));
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
