//! `dashcraft render`: refreshes a document and renders report pages
//! headlessly to PNG (through resvg, with the bundled fonts only) or SVG.
//!
//! Output is deterministic: the same inputs give byte-identical files on
//! every OS. Pass `--now` to pin the clock that M's `DateTime.LocalNow()` /
//! `DateTimeZone.UtcNow()` read (e.g. a "Last Refreshed" table); timings are
//! only reported in `--json`, never drawn.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, anyhow, bail};
use chrono::{DateTime, FixedOffset, Utc};
use pbi_core::ReportSource;
use pbi_model::{Page, Report, Visual, dax_column_ref, names_eq};
use pbi_render::png::{PngOptions, render_png};
use pbi_render::query::{
    InteractionState, QueryEnv, SlicerSelection, is_slicer_type, slicer_fields,
};
use pbi_render::scene::Color;
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::{PageRenderOptions, RenderedPage, render_report_page};
use pbi_session::{Clock, FixedClock, Session};
use serde::Serialize;
use serde_json::Value as Json;

use crate::common::{Context, Status, print_json, with_stdout, write_file};
use crate::data::{
    DataArgs, RefreshCounts, find_report, open_session_with_clock, print_problems, refresh, runtime,
};

/// Version of the `render --json` schema.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest `--scale`.
pub const MAX_SCALE: f32 = 16.0;
/// Longest page-name part of an `--out-dir` file name.
const SLUG_LEN: usize = 60;

/// What `--out-dir` writes per page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum FormatArg {
    /// `<NN>-<page>.png`
    #[default]
    Png,
    /// `<NN>-<page>.svg`
    Svg,
    /// Both.
    Both,
}

#[derive(Debug, clap::Args)]
#[command(group(
    clap::ArgGroup::new("output")
        .required(true)
        .multiple(true)
        .args(["png", "svg", "out_dir"])
))]
pub struct RenderArgs {
    /// File or folder: .pbip, .pbix, .pbit, *.Report, …
    pub path: PathBuf,
    /// The report to render (name as in `inspect`, case-insensitive, or a
    /// unique part of it). Default: the first report.
    #[arg(long, value_name = "NAME")]
    pub report: Option<String>,
    /// The page: internal name, display name (case-insensitive), 1-based
    /// index, or a unique part of the display name. Default: the report's
    /// active page.
    #[arg(long, value_name = "PAGE", conflicts_with = "all_pages")]
    pub page: Option<String>,
    /// Render every page of the report (with --out-dir).
    #[arg(long)]
    pub all_pages: bool,
    /// Write the page as a PNG.
    #[arg(long, value_name = "FILE", conflicts_with = "out_dir")]
    pub png: Option<PathBuf>,
    /// Write the page as an SVG.
    #[arg(long, value_name = "FILE", conflicts_with = "out_dir")]
    pub svg: Option<PathBuf>,
    /// Write `<NN>-<page name>.png` (see --format) per page into DIR
    /// (created if missing); NN is the 1-based page index.
    #[arg(long, value_name = "DIR")]
    pub out_dir: Option<PathBuf>,
    /// What --out-dir writes per page (default: png).
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub format: Option<FormatArg>,
    /// PNG pixels per page unit (2: a 1280×720 page becomes 2560×1440).
    #[arg(long, value_name = "N", default_value_t = 2.0, value_parser = parse_scale)]
    pub scale: f32,
    /// Select slicer values: `SLICER=V1,V2` (or `SLICER!=V1,V2` to select
    /// everything else; `SLICER=` for all). SLICER is the slicer's visual
    /// name or its field (display name, column, `Table[Column]`),
    /// case-insensitive; `(Blank)` selects BLANK. Repeatable; replaces the
    /// slicer's saved selection.
    #[arg(long, value_name = "SLICER=VALUES")]
    pub slicer: Vec<String>,
    /// The time M sees as "now" (RFC 3339, e.g. 2026-10-21T21:35:00Z), for
    /// reproducible "Last Refreshed" values. Default: the system clock.
    #[arg(long, value_name = "RFC3339", value_parser = parse_now)]
    pub now: Option<DateTime<FixedOffset>>,
    /// Overwrite existing output files.
    #[arg(long)]
    pub force: bool,
    /// Print per-visual status as JSON (`schemaVersion` 1).
    #[arg(long)]
    pub json: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
    #[command(flatten)]
    pub data: DataArgs,
}

fn parse_scale(text: &str) -> Result<f32, String> {
    let v: f32 = text
        .trim()
        .parse()
        .map_err(|_| format!("'{text}' is not a number"))?;
    if v.is_finite() && v > 0.0 && v <= MAX_SCALE {
        Ok(v)
    } else {
        Err(format!("must be greater than 0 and at most {MAX_SCALE}"))
    }
}

fn parse_now(text: &str) -> Result<DateTime<FixedOffset>, String> {
    DateTime::parse_from_rfc3339(text.trim()).map_err(|e| {
        format!("'{text}' is not an RFC 3339 date-time ({e}), e.g. 2026-10-21T21:35:00Z")
    })
}

/// Root of `render --json`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOut {
    pub schema_version: u32,
    /// The path as given on the command line.
    pub source: String,
    /// The rendered report.
    pub report: String,
    /// Refresh outcome; `null` without a local semantic model.
    pub refresh: Option<RefreshCounts>,
    /// The clock M saw (`--now`), RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    /// The refresh succeeded and no visual failed.
    pub ok: bool,
    pub pages: Vec<PageOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOut {
    /// 1-based position in the report.
    pub index: usize,
    /// Internal name.
    pub name: String,
    pub display_name: String,
    /// Page size in page units.
    pub width: f64,
    pub height: f64,
    pub outputs: Vec<OutputOut>,
    /// Query + render + encode wall time.
    pub duration_ms: u64,
    /// Slicer selections applied (saved in the report, or from --slicer).
    pub slicers: Vec<SlicerOut>,
    pub summary: VisualCounts,
    /// Every visual of the page, in page order.
    pub visuals: Vec<VisualOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputOut {
    /// `png` or `svg`.
    pub format: &'static str,
    pub path: String,
    /// Pixel size (PNG only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlicerOut {
    /// Slicer visual name.
    pub visual: String,
    /// Selected values (`null` = BLANK); one array per value for slicers
    /// on several fields.
    pub values: Vec<Json>,
    /// Everything except `values`.
    pub inverted: bool,
    /// A saved non-categorical selection (range, …) applies too.
    pub condition: bool,
    /// `report` (saved selection) or `cli` (--slicer).
    pub origin: &'static str,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualCounts {
    pub visuals: usize,
    pub ok: usize,
    /// Rendered, with filters not applied (or applied approximately).
    pub warnings: usize,
    pub errors: usize,
    pub hidden: usize,
    /// Data cut at the query limits.
    pub truncated: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualOut {
    pub name: String,
    pub visual_type: String,
    /// `ok`, `warning`, `error` or `hidden`.
    pub status: &'static str,
    /// The query error (the visual shows ⚠).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub query_micros: u64,
    pub render_micros: u64,
}

/// One file to write.
#[derive(Debug, Clone)]
struct Target {
    kind: Kind,
    path: PathBuf,
    overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Png,
    Svg,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Svg => "svg",
        }
    }
}

/// A parsed `--slicer`.
#[derive(Debug, Clone)]
struct SlicerSpec {
    raw: String,
    target: String,
    /// `None` = BLANK.
    values: Vec<Option<String>>,
    inverted: bool,
}

impl SlicerSpec {
    fn selection(&self) -> anyhow::Result<SlicerSelection> {
        let values: Vec<Json> = self
            .values
            .iter()
            .map(|v| v.as_ref().map_or(Json::Null, |s| Json::String(s.clone())))
            .collect();
        serde_json::from_value(serde_json::json!({ "values": values, "inverted": self.inverted }))
            .map_err(|e| anyhow!("--slicer {:?}: {e}", self.raw))
    }
}

fn parse_slicer(text: &str) -> anyhow::Result<SlicerSpec> {
    let Some(eq) = text.find('=') else {
        bail!("--slicer {text:?}: expected SLICER=VALUE[,VALUE…] (or SLICER!=…)");
    };
    let (head, values) = (&text[..eq], &text[eq + 1..]);
    let (target, inverted) = match head.strip_suffix('!') {
        Some(t) => (t, true),
        None => (head, false),
    };
    let target = target.trim();
    if target.is_empty() {
        bail!("--slicer {text:?}: the slicer name is empty");
    }
    let values = values
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| (!v.eq_ignore_ascii_case("(blank)")).then(|| v.to_owned()))
        .collect();
    Ok(SlicerSpec {
        raw: text.to_owned(),
        target: target.to_owned(),
        values,
        inverted,
    })
}

pub fn run(args: &RenderArgs, ctx: &Context) -> anyhow::Result<Status> {
    if args.out_dir.is_none() {
        if args.all_pages {
            bail!("--all-pages writes one file per page: use --out-dir DIR");
        }
        if args.format.is_some() {
            bail!("--format applies to --out-dir; with --png/--svg the flag names the format");
        }
    }
    let specs = args
        .slicer
        .iter()
        .map(|s| parse_slicer(s))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let clock = args.now.map(|t| {
        Arc::new(FixedClock {
            now: t.with_timezone(&Utc),
            offset: *t.offset(),
        }) as Arc<dyn Clock>
    });
    let session = open_session_with_clock(&args.path, &args.data, clock)?;
    let doc = session.document();
    if doc.reports.is_empty() {
        bail!("{} has no report to render", args.path.display());
    }
    let report_index = match &args.report {
        Some(name) => find_report(&session, name)?,
        None => {
            if doc.reports.len() > 1 {
                ctx.note(format!(
                    "rendering report '{}' (the document has {} reports; pick one with --report)",
                    doc.reports[0].name,
                    doc.reports.len()
                ));
            }
            0
        }
    };
    let report = &doc.reports[report_index];
    if report.pages.is_empty() {
        bail!("report '{}' has no pages", report.name);
    }
    let pages: Vec<usize> = if args.all_pages {
        (0..report.pages.len()).collect()
    } else {
        vec![find_page(report, args.page.as_deref())?]
    };
    let targets = pages
        .iter()
        .map(|&i| targets(args, i, &report.pages[i]))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let states = interaction_states(report, &pages, &specs, ctx)?;

    let started = Instant::now();
    let rt = runtime()?;
    let (refresh_counts, refresh_failed) = if session.has_model() {
        let report = refresh(&rt, &session, None, &args.data, ctx)?;
        (
            Some(RefreshCounts::of(&report)),
            print_problems(ctx, &report),
        )
    } else {
        ctx.warn(format!(
            "{} has no local semantic model; visuals that query data show ⚠",
            args.path.display()
        ));
        (None, false)
    };
    tracing::info!("refreshed in {} ms", millis(started));

    let theme = theme(&session, report_index);
    let env = QueryEnv::from_session(&session);
    let mut out_pages = Vec::with_capacity(pages.len());
    for ((&index, targets), (state, slicers)) in pages.iter().zip(&targets).zip(states) {
        let page = &report.pages[index];
        let started = Instant::now();
        let rendered = render_report_page(
            &env,
            report,
            page,
            &theme,
            &state,
            &PageRenderOptions::default(),
        );
        let outputs = write_outputs(&rendered, targets, args.scale)?;
        let duration_ms = millis(started);
        report_visuals(ctx, page, &rendered);
        out_pages.push(page_out(
            index,
            page,
            outputs,
            duration_ms,
            slicers,
            &rendered,
        ));
    }

    let visual_failed = out_pages.iter().any(|p| p.summary.errors > 0);
    if args.json || args.pretty {
        let out = RenderOut {
            schema_version: SCHEMA_VERSION,
            source: args.path.to_string_lossy().into_owned(),
            report: report.name.clone(),
            refresh: refresh_counts,
            now: args.now.map(|t| t.to_rfc3339()),
            ok: !refresh_failed && !visual_failed,
            pages: out_pages,
        };
        print_json(&out, args.pretty)?;
    } else {
        with_stdout(|w| {
            for p in &out_pages {
                for o in &p.outputs {
                    let size = match (o.width, o.height) {
                        (Some(w), Some(h)) => format!("{w}x{h}, "),
                        _ => String::new(),
                    };
                    writeln!(
                        w,
                        "{}: {} ({size}{} visuals, {} errors)",
                        p.display_name, o.path, p.summary.visuals, p.summary.errors
                    )?;
                }
            }
            Ok(())
        })?;
    }
    Ok(if refresh_failed || visual_failed {
        Status::ErrorDiagnostics
    } else {
        Status::Ok
    })
}

fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The report's theme (built-in defaults for legacy layouts).
fn theme(session: &Session, report_index: usize) -> ThemeResolver {
    match session.opened().report_sources.get(report_index) {
        Some(ReportSource::Pbir { report, .. }) => ThemeResolver::from_pbir(report),
        _ => ThemeResolver::default(),
    }
}

/// The page `spec` names: internal name, display name (case-insensitive),
/// 1-based index, or a unique part of the display name; the active (else
/// first) page when `None`.
fn find_page(report: &Report, spec: Option<&str>) -> anyhow::Result<usize> {
    let pages = &report.pages;
    let Some(spec) = spec.map(str::trim) else {
        return Ok(report
            .active_page
            .as_deref()
            .and_then(|a| pages.iter().position(|p| p.name == a))
            .unwrap_or(0));
    };
    let list = || {
        pages
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{} '{}' [{}]", i + 1, p.display_name, p.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(i) = pages.iter().position(|p| p.name == spec) {
        return Ok(i);
    }
    if let Some(i) = pages.iter().position(|p| names_eq(&p.display_name, spec)) {
        return Ok(i);
    }
    if let Ok(n) = spec.parse::<usize>() {
        if (1..=pages.len()).contains(&n) {
            return Ok(n - 1);
        }
        bail!(
            "--page {n}: report '{}' has {} page(s): {}",
            report.name,
            pages.len(),
            list()
        );
    }
    let needle = spec.to_lowercase();
    let mut partial = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.display_name.to_lowercase().contains(&needle));
    match (partial.next(), partial.next()) {
        (Some((i, _)), None) => Ok(i),
        (Some(_), Some(_)) => bail!("--page '{spec}' matches several pages: {}", list()),
        _ => bail!(
            "no page '{spec}' in report '{}'; pages: {}",
            report.name,
            list()
        ),
    }
}

/// The files to write for page `index`, checked before any work: a file
/// that exists (and is not empty) needs --force.
fn targets(args: &RenderArgs, index: usize, page: &Page) -> anyhow::Result<Vec<Target>> {
    let mut wanted: Vec<(Kind, PathBuf)> = Vec::new();
    if let Some(dir) = &args.out_dir {
        if let Ok(meta) = fs::metadata(dir)
            && !meta.is_dir()
        {
            bail!(
                "{} is not a folder; --out-dir takes a folder",
                dir.display()
            );
        }
        let stem = format!("{:02}-{}", index + 1, slug(&page.display_name));
        let format = args.format.unwrap_or_default();
        if matches!(format, FormatArg::Png | FormatArg::Both) {
            wanted.push((Kind::Png, dir.join(format!("{stem}.png"))));
        }
        if matches!(format, FormatArg::Svg | FormatArg::Both) {
            wanted.push((Kind::Svg, dir.join(format!("{stem}.svg"))));
        }
    } else {
        if let Some(p) = &args.png {
            wanted.push((Kind::Png, p.clone()));
        }
        if let Some(p) = &args.svg {
            wanted.push((Kind::Svg, p.clone()));
        }
        if let (Some(png), Some(svg)) = (&args.png, &args.svg)
            && png == svg
        {
            bail!("--png and --svg name the same file {}", png.display());
        }
    }
    wanted
        .into_iter()
        .map(|(kind, path)| {
            let overwrite = match fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => {
                    bail!(
                        "{} is a folder; --{} takes a file",
                        path.display(),
                        kind.name()
                    )
                }
                Ok(meta) if meta.len() > 0 && !args.force => bail!(
                    "{} already exists; use --force to overwrite it",
                    path.display()
                ),
                Ok(_) => true,
                Err(_) => args.force,
            };
            Ok(Target {
                kind,
                path,
                overwrite,
            })
        })
        .collect()
}

/// A file-name-safe page name: ASCII letters and digits, other runs as `-`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= SLUG_LEN {
            break;
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "page".to_owned()
    } else {
        out.to_owned()
    }
}

/// Labels a slicer answers to: its fields' display names, column names and
/// `Table[Column]` references.
fn slicer_labels(v: &Visual) -> Vec<String> {
    let mut labels = Vec::new();
    for p in v.projections_for("Values") {
        labels.push(p.label().to_owned());
        if let Some(c) = p.field.property() {
            labels.push(c.to_owned());
            if let Some(t) = p.field.entity() {
                labels.push(dax_column_ref(t, c));
                labels.push(format!("{t}.{c}"));
            }
        }
    }
    labels
}

/// The slicers of `page` that `target` names (visual name first).
fn matching_slicers<'a>(page: &'a Page, target: &str) -> Vec<&'a Visual> {
    let slicers = page
        .visuals
        .iter()
        .filter(|v| is_slicer_type(&v.visual_type));
    if let Some(v) = slicers.clone().find(|v| v.name == target) {
        return vec![v];
    }
    slicers
        .filter(|v| slicer_labels(v).iter().any(|l| names_eq(l, target)))
        .collect()
}

/// Every page's interaction state: the saved slicer selections, replaced by
/// `--slicer` where one names a slicer of the page. A `--slicer` that names
/// no slicer on any rendered page, or several on one page, is an error.
fn interaction_states(
    report: &Report,
    pages: &[usize],
    specs: &[SlicerSpec],
    ctx: &Context,
) -> anyhow::Result<Vec<(InteractionState, Vec<SlicerOut>)>> {
    let mut used = vec![false; specs.len()];
    let mut out = Vec::with_capacity(pages.len());
    for &i in pages {
        let page = &report.pages[i];
        let (mut state, problems) = InteractionState::from_page(page);
        for p in problems {
            ctx.warn(format!("page '{}': saved {p}", page.display_name));
        }
        let mut cli = Vec::new();
        for (spec, used) in specs.iter().zip(used.iter_mut()) {
            let matches = matching_slicers(page, &spec.target);
            match matches.as_slice() {
                [] => {}
                [v] => {
                    *used = true;
                    if spec.values.is_empty() && !spec.inverted {
                        state.clear_slicer(&v.name);
                    } else {
                        state.select(v.name.clone(), spec.selection()?);
                    }
                    cli.push(v.name.clone());
                }
                several => bail!(
                    "--slicer {:?} matches several slicers on page '{}': {}",
                    spec.raw,
                    page.display_name,
                    several
                        .iter()
                        .map(|v| v.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        let slicers = state
            .slicer_selections
            .iter()
            .map(|(name, sel)| SlicerOut {
                visual: name.clone(),
                values: selection_values(sel),
                inverted: sel.inverted,
                condition: sel.condition.is_some(),
                origin: if cli.contains(name) { "cli" } else { "report" },
            })
            .collect();
        out.push((state, slicers));
    }
    if let Some(spec) = specs.iter().zip(&used).find(|(_, u)| !**u).map(|(s, _)| s) {
        let names: Vec<String> = pages
            .iter()
            .flat_map(|&i| report.pages[i].visuals.iter())
            .filter(|v| is_slicer_type(&v.visual_type))
            .map(|v| {
                let field = slicer_fields(v)
                    .first()
                    .and_then(|f| Some(dax_column_ref(f.entity()?, f.property()?)))
                    .unwrap_or_default();
                format!("{} ({field})", v.name)
            })
            .collect();
        bail!(
            "--slicer {:?}: no slicer '{}' on the rendered page(s); slicers: {}",
            spec.raw,
            spec.target,
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        );
    }
    Ok(out)
}

/// The selected tuples as JSON (through the selection's own serde form),
/// one-field tuples as bare values.
fn selection_values(sel: &SlicerSelection) -> Vec<Json> {
    let tuples = match serde_json::to_value(sel) {
        Ok(Json::Object(mut o)) => o.remove("values"),
        _ => None,
    };
    let Some(Json::Array(tuples)) = tuples else {
        return Vec::new();
    };
    tuples
        .into_iter()
        .map(|t| match t {
            Json::Array(mut a) if a.len() == 1 => a.pop().unwrap_or(Json::Null),
            other => other,
        })
        .collect()
}

/// Encodes and writes the page's files.
fn write_outputs(
    rendered: &RenderedPage,
    targets: &[Target],
    scale: f32,
) -> anyhow::Result<Vec<OutputOut>> {
    let mut outputs = Vec::with_capacity(targets.len());
    for t in targets {
        let (bytes, size) = match t.kind {
            Kind::Png => {
                let png = render_png(
                    &rendered.scene,
                    &PngOptions {
                        scale,
                        background: Some(Color::WHITE),
                    },
                )
                .map_err(|e| anyhow!(e).context(format!("cannot render {}", t.path.display())))?;
                let size = png_size(&png);
                (png, size)
            }
            Kind::Svg => (
                to_svg(&rendered.scene, &SvgOptions::default()).into_bytes(),
                None,
            ),
        };
        write_file(&t.path, t.overwrite, |w| {
            w.write_all(&bytes)
                .with_context(|| format!("cannot write {}", t.path.display()))
        })?;
        outputs.push(OutputOut {
            format: t.kind.name(),
            path: t.path.to_string_lossy().into_owned(),
            width: size.map(|s| s.0),
            height: size.map(|s| s.1),
            bytes: bytes.len() as u64,
        });
    }
    Ok(outputs)
}

/// Width and height from a PNG's `IHDR`.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let w = png.get(16..20)?.try_into().ok().map(u32::from_be_bytes)?;
    let h = png.get(20..24)?.try_into().ok().map(u32::from_be_bytes)?;
    Some((w, h))
}

/// Visual errors on stderr; warnings at `-v`.
fn report_visuals(ctx: &Context, page: &Page, rendered: &RenderedPage) {
    for v in &rendered.visuals {
        if let Some(e) = &v.error {
            ctx.warn(format!(
                "page '{}': visual {} ({}) failed: {e}",
                page.display_name, v.name, v.visual_type
            ));
        }
        for w in &v.warnings {
            tracing::info!(
                "page '{}': visual {} ({}): {w}",
                page.display_name,
                v.name,
                v.visual_type
            );
        }
    }
}

fn page_out(
    index: usize,
    page: &Page,
    outputs: Vec<OutputOut>,
    duration_ms: u64,
    slicers: Vec<SlicerOut>,
    rendered: &RenderedPage,
) -> PageOut {
    let mut summary = VisualCounts::default();
    let visuals: Vec<VisualOut> = rendered
        .visuals
        .iter()
        .map(|v| {
            let status = if v.hidden {
                "hidden"
            } else if v.error.is_some() {
                "error"
            } else if !v.warnings.is_empty() {
                "warning"
            } else {
                "ok"
            };
            summary.visuals += 1;
            match status {
                "hidden" => summary.hidden += 1,
                "error" => summary.errors += 1,
                "warning" => summary.warnings += 1,
                _ => summary.ok += 1,
            }
            summary.truncated += usize::from(v.truncated);
            VisualOut {
                name: v.name.clone(),
                visual_type: v.visual_type.clone(),
                status,
                error: v.error.clone(),
                warnings: v.warnings.clone(),
                truncated: v.truncated,
                query_micros: v.query_micros,
                render_micros: v.render_micros,
            }
        })
        .collect();
    PageOut {
        index: index + 1,
        name: page.name.clone(),
        display_name: page.display_name.clone(),
        width: page.width,
        height: page.height,
        outputs,
        duration_ms,
        slicers,
        summary,
        visuals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slicer_specs_parse() {
        let s = parse_slicer("Loyalty Tier=Gold, (Blank) ,").unwrap();
        assert_eq!(s.target, "Loyalty Tier");
        assert_eq!(s.values, vec![Some("Gold".to_owned()), None]);
        assert!(!s.inverted);
        let s = parse_slicer("Stores[Channel]!=Online").unwrap();
        assert_eq!((s.target.as_str(), s.inverted), ("Stores[Channel]", true));
        assert!(parse_slicer("Status=").unwrap().values.is_empty());
        assert!(parse_slicer("no value").is_err());
        assert!(parse_slicer(" =A").is_err());
    }

    #[test]
    fn file_names_are_slugs() {
        assert_eq!(slug("Store Performance"), "Store-Performance");
        assert_eq!(
            slug("  Stores / Regions: Not In Plan! "),
            "Stores-Regions-Not-In-Plan"
        );
        assert_eq!(slug("ü…"), "page");
        assert!(slug(&"x".repeat(200)).len() <= SLUG_LEN);
    }

    #[test]
    fn scale_and_now_are_validated() {
        assert_eq!(parse_scale("1.5"), Ok(1.5));
        assert!(parse_scale("0").is_err());
        assert!(parse_scale("NaN").is_err());
        assert!(parse_scale("100").is_err());
        assert!(parse_now("2026-10-21T21:35:00Z").is_ok());
        assert!(parse_now("yesterday").is_err());
    }
}
