//! Performance scenarios of editing and saving, run through `Registry::execute` like the app,
//! `serve` and MCP (see `perf/budgets.toml` and `docs/perf.md`; run them with `cargo xtask perf`):
//!
//! - P09 `edit-measure-requery`: change `Total Sales` of the Acme Outfitters example (refreshed
//!   offline from its CSVs), then re-render the Executive Summary page on the canvas.
//! - P10 `save-pbip-acme`: one measure edit, then `doc.save` into a copy of the example under
//!   `target/tmp/`.
//! - P11 `refresh-acme-end-to-end`: open the example, refresh it from its CSVs and render the
//!   first page of the first report.
//! - P13 `tooltip-page-render`: hover the segments of the Store Performance stacked chart in
//!   turn; `canvas.tooltipPage` queries and renders the Store Tooltip page for each.

#[path = "../../../perf/harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use harness::{Args, Outcome, ms_since, sample};
use pbi_ops::{CallerKind, Ctx, Registry, Workspace};
use serde_json::{Value, json};

const ACME: &str = "examples/acme-outfitters";
const TOTAL_SALES: &str = "SUM(Orders[Line Amount])";
/// Same values (BLANK stays BLANK), different text: every edit commits a new revision.
const TOTAL_SALES_EDITED: &str = "SUM(Orders[Line Amount]) * 1.0";

fn main() -> ExitCode {
    harness::main(
        env!("CARGO_MANIFEST_DIR"),
        &[
            ("P09", edit_measure_requery),
            ("P10", save_pbip_acme),
            ("P11", refresh_acme_end_to_end),
            ("P13", tooltip_page_render),
        ],
    )
}

/// One workspace driven through the registry with a CLI caller.
struct Ops {
    rt: tokio::runtime::Runtime,
    registry: Registry,
    ws: Workspace,
    ctx: Ctx,
}

impl Ops {
    fn new() -> Result<Self, String> {
        Ok(Self {
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?,
            registry: Registry::default_catalog(),
            ws: Workspace::new(),
            ctx: Ctx::new(CallerKind::Cli),
        })
    }

    fn run(&self, id: &str, params: Value) -> Result<Value, String> {
        self.rt
            .block_on(self.registry.execute(&self.ws, id, params, &self.ctx))
            .map_err(|e| format!("{id}: {e}"))
    }

    fn open(&self, pbip: &Path, blob_map: Option<&Path>) -> Result<String, String> {
        let out = self.run(
            "doc.open",
            json!({ "path": pbip, "blobMap": blob_map, "autoRefresh": false }),
        )?;
        out["docId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "doc.open: no docId".to_owned())
    }

    /// Refreshes every table; returns how many there are.
    fn refresh(&self, doc: &str) -> Result<usize, String> {
        let out = self.run("data.refresh", json!({ "docId": doc }))?;
        if out["ok"] != true {
            return Err(format!("data.refresh failed: {}", out["summary"]));
        }
        Ok(out["tables"].as_array().map_or(0, Vec::len))
    }

    /// Sets `Total Sales` to the edited text (even `i`) or back; `Err` unless the edit
    /// committed.
    fn edit_total_sales(&self, doc: &str, i: usize) -> Result<(), String> {
        let expression = if i.is_multiple_of(2) {
            TOTAL_SALES_EDITED
        } else {
            TOTAL_SALES
        };
        let out = self.run(
            "model.measure.update",
            json!({
                "docId": doc,
                "table": "Orders",
                "measure": "Total Sales",
                "expression": expression,
            }),
        )?;
        if out["changed"] != true {
            return Err("model.measure.update did not change the measure".to_owned());
        }
        Ok(())
    }

    /// Renders a page on the canvas; returns the SVG size in bytes.
    fn canvas(&self, doc: &str, report: Option<&str>, page: Option<&str>) -> Result<usize, String> {
        let out = self.run(
            "canvas.render",
            json!({ "docId": doc, "report": report, "page": page }),
        )?;
        out["svg"]
            .as_str()
            .map(str::len)
            .ok_or_else(|| "canvas.render: no svg".to_owned())
    }
}

/// The example's `.pbip` and `blob-map.json`, or a skip when they are missing.
fn acme(args: &Args) -> Result<(PathBuf, PathBuf), Outcome> {
    let dir = args.reference.join(ACME);
    let (pbip, blob_map) = (dir.join("AcmeOutfitters.pbip"), dir.join("blob-map.json"));
    for path in [&pbip, &blob_map] {
        if !path.exists() {
            return Err(Outcome::missing(path));
        }
    }
    Ok((pbip, blob_map))
}

fn edit_measure_requery(args: &Args) -> Result<Outcome, String> {
    let (pbip, blob_map) = match acme(args) {
        Ok(paths) => paths,
        Err(skip) => return Ok(skip),
    };
    let ops = Ops::new()?;
    let doc = ops.open(&pbip, Some(&blob_map))?;
    ops.refresh(&doc)?;
    let (report, page) = (Some("Overview"), Some("Executive Summary"));
    let svg_bytes = ops.canvas(&doc, report, page)?;
    let (mut edit_ms, mut render_ms) = (Vec::new(), Vec::new());
    let samples = sample(args.warmup, args.iterations, |i| {
        let t = Instant::now();
        ops.edit_total_sales(&doc, i)?;
        let edited = ms_since(t);
        let t = Instant::now();
        ops.canvas(&doc, report, page)?;
        if i >= args.warmup {
            edit_ms.push(edited);
            render_ms.push(ms_since(t));
        }
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("svg_bytes", svg_bytes)
        .with("edit_median_ms", median(&mut edit_ms))
        .with("render_median_ms", median(&mut render_ms)))
}

fn save_pbip_acme(args: &Args) -> Result<Outcome, String> {
    let (pbip, _) = match acme(args) {
        Ok(paths) => paths,
        Err(skip) => return Ok(skip),
    };
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let tmp = tempfile::Builder::new()
        .prefix("perf-p10-")
        .tempdir_in(&scratch)
        .map_err(|e| e.to_string())?;
    let source = pbip.parent().ok_or("no example folder")?;
    copy_dir(source, tmp.path())?;
    let ops = Ops::new()?;
    let doc = ops.open(&tmp.path().join("AcmeOutfitters.pbip"), None)?;
    let (mut save_ms, mut written) = (Vec::new(), 0);
    let samples = sample(args.warmup, args.iterations, |i| {
        ops.edit_total_sales(&doc, i)?;
        let t = Instant::now();
        let out = ops.run("doc.save", json!({ "docId": doc }))?;
        let saved = ms_since(t);
        written = out["written"].as_array().map_or(0, Vec::len);
        if written == 0 {
            return Err("doc.save wrote nothing".to_owned());
        }
        if i >= args.warmup {
            save_ms.push(saved);
        }
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("files_written", written)
        .with("save_median_ms", median(&mut save_ms)))
}

fn refresh_acme_end_to_end(args: &Args) -> Result<Outcome, String> {
    let (pbip, blob_map) = match acme(args) {
        Ok(paths) => paths,
        Err(skip) => return Ok(skip),
    };
    let ops = Ops::new()?;
    let (mut open_ms, mut refresh_ms, mut render_ms) = (0.0, 0.0, 0.0);
    let mut tables = 0;
    let samples = sample(args.warmup, args.iterations, |_| {
        let t = Instant::now();
        let doc = ops.open(&pbip, Some(&blob_map))?;
        open_ms = ms_since(t);
        let t = Instant::now();
        tables = ops.refresh(&doc)?;
        refresh_ms = ms_since(t);
        let t = Instant::now();
        ops.canvas(&doc, None, Some("1"))?;
        render_ms = ms_since(t);
        ops.run("doc.close", json!({ "docId": doc }))?;
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("tables", tables)
        .with("last_open_ms", open_ms)
        .with("last_refresh_ms", refresh_ms)
        .with("last_render_ms", render_ms))
}

fn tooltip_page_render(args: &Args) -> Result<Outcome, String> {
    let (pbip, blob_map) = match acme(args) {
        Ok(paths) => paths,
        Err(skip) => return Ok(skip),
    };
    let ops = Ops::new()?;
    let doc = ops.open(&pbip, Some(&blob_map))?;
    ops.refresh(&doc)?;
    let at = json!({ "docId": doc, "report": "Stores", "page": "Store Performance" });
    let page = ops.run("canvas.render", at.clone())?;
    let chart = page["visuals"]
        .as_array()
        .and_then(|v| v.iter().find(|v| v["visualType"] == "stackedColumnChart"))
        .and_then(|v| v["name"].as_str())
        .ok_or("no stacked column chart on Store Performance")?
        .to_owned();
    let prefix = format!("{chart}|dataPoint|");
    let hits: Vec<String> = page["hitRegions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["id"].as_str())
        .filter(|id| id.starts_with(&prefix))
        .map(str::to_owned)
        .collect();
    if hits.is_empty() {
        return Err("the stacked chart has no data points".to_owned());
    }
    let mut svg_bytes = 0;
    let samples = sample(args.warmup, args.iterations, |i| {
        let mut params = at.clone();
        params["hitId"] = json!(hits[i % hits.len()]);
        let out = ops.run("canvas.tooltipPage", params)?;
        svg_bytes = out["tooltip"]["svg"]
            .as_str()
            .map(str::len)
            .ok_or("canvas.tooltipPage: no tooltip page")?;
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("points", hits.len())
        .with("svg_bytes", svg_bytes))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
}
