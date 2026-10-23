//! `render.page`, `render.visual`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use pbi_render::png::{PngOptions, render_png};
use pbi_render::query::{InteractionState, QueryEnv, QueryLimits, SlicerSelection};
use pbi_render::scene::{Color, Group, HitKind, Node, Scene, Transform};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::visuals::{PageRenderOptions, RenderedPage, render_report_page};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{BASE, loaded_doc, millis};
use crate::canvas::selection_of;
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::dto::RefreshCounts;
use crate::error::OpError;
use crate::files::{check_output_file, write_bytes};
use crate::resolve;
use crate::workspace::{DocEntry, Workspace};

/// Largest scale (PNG pixels per page unit).
pub const MAX_SCALE: f32 = 16.0;
/// Largest inline result (SVG text or PNG bytes before base64); pass
/// `outputPath` for bigger renders.
pub const MAX_INLINE_BYTES: usize = 8 * 1024 * 1024;

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RenderFormat {
    /// A standalone SVG document (deterministic, `data-hit` attributes).
    #[default]
    Svg,
    /// A PNG (white background, bundled fonts; byte-identical across OSes).
    Png,
}

/// A slicer selection for the render (replaces the saved selection).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlicerParam {
    /// The slicer's visual name.
    #[serde(default)]
    pub visual: Option<String>,
    /// Or the slicer's field: display name, column, `Table[Column]`
    /// (case-insensitive).
    #[serde(default)]
    pub field: Option<String>,
    /// Values to select (`null` = BLANK). Empty without `invert`: select
    /// all (clears the slicer).
    #[serde(default)]
    pub values: Vec<Value>,
    /// Select everything except `values`.
    #[serde(default)]
    pub invert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderPageParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report (default: the first).
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of
    /// the display name (default: the active page).
    #[serde(default)]
    pub page: Option<String>,
    /// `svg` (default) or `png`.
    #[serde(default)]
    pub format: RenderFormat,
    /// PNG pixels per page unit (default 1; greater than 0, at most 16).
    #[serde(default)]
    pub scale: Option<f32>,
    /// Slicer selections replacing the saved ones.
    #[serde(default)]
    pub slicers: Vec<SlicerParam>,
    /// Write the output to this file instead of returning it inline.
    #[serde(default)]
    pub output_path: Option<PathBuf>,
    /// Replace an existing `outputPath`.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderVisualParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The visual's name (see `report.visuals`).
    pub visual: String,
    #[serde(default)]
    pub format: RenderFormat,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub slicers: Vec<SlicerParam>,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
    #[serde(default)]
    pub overwrite: bool,
}

/// The rendered page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenderedPageInfo {
    /// 1-based position in the report.
    pub index: usize,
    pub name: String,
    pub display_name: String,
    /// Page size in page units.
    pub width: f64,
    pub height: f64,
}

/// Visual counts of a render.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisualCounts {
    pub visuals: usize,
    pub ok: usize,
    /// Rendered with filters not applied (or applied approximately).
    pub warnings: usize,
    pub errors: usize,
    pub hidden: usize,
    /// Data cut at the query limits.
    pub truncated: usize,
}

/// Status of one visual.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisualStatusOut {
    pub name: String,
    pub visual_type: String,
    /// `ok`, `warning`, `error` or `hidden`.
    pub status: String,
    /// The query error (the visual shows ⚠).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub query_micros: u64,
    pub render_micros: u64,
}

/// A slicer selection applied to the render.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlicerOut {
    pub visual: String,
    /// Selected values (`null` = BLANK); arrays for multi-field slicers.
    pub values: Vec<Value>,
    pub inverted: bool,
    /// A saved non-categorical condition (range, …) applies too.
    pub condition: bool,
    /// `report` (saved) or `params`.
    pub origin: String,
}

/// Result of `render.page` / `render.visual`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenderOut {
    /// No visual failed and the data loaded.
    pub ok: bool,
    pub report: String,
    pub page: RenderedPageInfo,
    /// The rendered visual (`render.visual`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<String>,
    /// `svg` or `png`.
    pub format: RenderFormat,
    /// The SVG document (inline SVG renders).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svg: Option<String>,
    /// The PNG, base64 (inline PNG renders).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png_base64: Option<String>,
    /// The written file (with `outputPath`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    /// Size of the SVG or PNG in bytes.
    pub bytes: usize,
    /// PNG size in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_height: Option<u32>,
    pub summary: VisualCounts,
    /// Every visual of the page (or the one visual), in page order.
    pub visuals: Vec<VisualStatusOut>,
    /// Slicer selections applied.
    pub slicers: Vec<SlicerOut>,
    /// What the data looked like (failed or never-loaded tables).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<RefreshCounts>,
    /// Query + render + encode wall time.
    pub duration_ms: u64,
}

struct Request {
    report: Option<String>,
    page: Option<String>,
    visual: Option<String>,
    format: RenderFormat,
    scale: Option<f32>,
    slicers: Vec<SlicerParam>,
    output_path: Option<PathBuf>,
    overwrite: bool,
}

fn scale_of(format: RenderFormat, scale: Option<f32>) -> Result<f32, OpError> {
    let scale = scale.unwrap_or(1.0);
    if format == RenderFormat::Svg {
        return Ok(scale);
    }
    if scale.is_finite() && scale > 0.0 && scale <= MAX_SCALE {
        Ok(scale)
    } else {
        Err(OpError::invalid_params(format!(
            "scale must be greater than 0 and at most {MAX_SCALE}"
        )))
    }
}

/// Saved selections replaced by `slicers`.
fn interaction_state(
    page: &pbi_model::Page,
    slicers: &[SlicerParam],
) -> Result<(InteractionState, Vec<SlicerOut>), OpError> {
    let (mut state, _problems) = InteractionState::from_page(page);
    let mut from_params = Vec::new();
    for s in slicers {
        let v = resolve::slicer(page, s.visual.as_deref(), s.field.as_deref())?;
        if s.values.is_empty() && !s.invert {
            state.clear_slicer(&v.name);
        } else {
            state.select(v.name.clone(), selection_of(s.values.clone(), s.invert)?);
        }
        from_params.push(v.name.clone());
    }
    let out = state
        .slicer_selections
        .iter()
        .map(|(name, sel)| SlicerOut {
            visual: name.clone(),
            values: selection_values(sel),
            inverted: sel.inverted,
            condition: sel.condition.is_some(),
            origin: if from_params.contains(name) {
                "params"
            } else {
                "report"
            }
            .to_owned(),
        })
        .collect();
    Ok((state, out))
}

/// The selected tuples as JSON, one-field tuples as bare values.
fn selection_values(sel: &SlicerSelection) -> Vec<Value> {
    let tuples = match serde_json::to_value(sel) {
        Ok(Value::Object(mut o)) => o.remove("values"),
        _ => None,
    };
    let Some(Value::Array(tuples)) = tuples else {
        return Vec::new();
    };
    tuples
        .into_iter()
        .map(|t| match t {
            Value::Array(mut a) if a.len() == 1 => a.pop().unwrap_or(Value::Null),
            other => other,
        })
        .collect()
}

/// Width and height from a PNG's `IHDR`.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let w = png.get(16..20)?.try_into().ok().map(u32::from_be_bytes)?;
    let h = png.get(20..24)?.try_into().ok().map(u32::from_be_bytes)?;
    Some((w, h))
}

/// The scene cropped to `visual`'s container.
fn crop(scene: Scene, visual: &str) -> Result<Scene, OpError> {
    let rect = scene
        .hit_regions()
        .into_iter()
        .find(|r| r.id.kind == HitKind::Visual && r.id.visual == visual)
        .map(|r| r.rect)
        .ok_or_else(|| {
            OpError::not_found(format!(
                "visual '{visual}' is hidden or has no container on the page"
            ))
        })?;
    let mut out = Scene::new(rect.width.max(1.0), rect.height.max(1.0));
    out.root = Group {
        transform: Some(Transform::translate(-rect.x, -rect.y)),
        children: vec![Node::Group(scene.root)],
        ..Group::default()
    };
    Ok(out)
}

fn visuals_out(
    rendered: &RenderedPage,
    only: Option<&str>,
) -> (Vec<VisualStatusOut>, VisualCounts) {
    let mut summary = VisualCounts::default();
    let visuals = rendered
        .visuals
        .iter()
        .filter(|v| only.is_none_or(|o| v.name == o))
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
            VisualStatusOut {
                name: v.name.clone(),
                visual_type: v.visual_type.clone(),
                status: status.to_owned(),
                error: v.error.clone(),
                warnings: v.warnings.clone(),
                truncated: v.truncated,
                query_micros: v.query_micros,
                render_micros: v.render_micros,
            }
        })
        .collect();
    (visuals, summary)
}

async fn render(
    ws: &Workspace,
    entry: Arc<DocEntry>,
    req: Request,
    ctx: &Ctx,
) -> Result<RenderOut, OpError> {
    let scale = scale_of(req.format, req.scale)?;
    let session = entry.session().clone();
    let ri = resolve::report_index(&session, req.report.as_deref())?;
    let pi = resolve::page_index(resolve::report_at(&session, ri)?, req.page.as_deref())?;
    let (state, slicers) = {
        let report = resolve::report_at(&session, ri)?;
        let page = report
            .pages
            .get(pi)
            .ok_or_else(|| OpError::not_found("no such page"))?;
        if let Some(v) = &req.visual {
            resolve::visual(page, v)?;
        }
        interaction_state(page, &req.slicers)?
    };
    let output = req
        .output_path
        .as_deref()
        .map(|p| check_output_file(ws.policy(), p, req.overwrite))
        .transpose()?;
    let data = session
        .has_model()
        .then(|| RefreshCounts::of_session(&session));
    let cancel = ctx.cancel_token().child_token();
    let visual = req.visual.clone();
    let format = req.format;
    let started = Instant::now();
    let (bytes, rendered) = tokio::task::spawn_blocking(move || {
        let report = resolve::report_at(&session, ri)?;
        let page = report
            .pages
            .get(pi)
            .ok_or_else(|| OpError::not_found("no such page"))?;
        let theme = resolve::theme(&session, ri);
        let env = QueryEnv::from_session(&session);
        let opts = PageRenderOptions {
            limits: QueryLimits {
                cancel: cancel.clone(),
                ..QueryLimits::default()
            },
            ..PageRenderOptions::default()
        };
        let mut rendered = render_report_page(&env, report, page, &theme, &state, &opts);
        if cancel.is_cancelled() {
            return Err(OpError::cancelled());
        }
        let scene = std::mem::replace(&mut rendered.scene, Scene::new(0.0, 0.0));
        let scene = match &visual {
            Some(v) => crop(scene, v)?,
            None => scene,
        };
        let bytes = match format {
            RenderFormat::Svg => to_svg(&scene, &SvgOptions::default()).into_bytes(),
            RenderFormat::Png => render_png(
                &scene,
                &PngOptions {
                    scale,
                    background: Some(Color::WHITE),
                },
            )
            .map_err(|e| OpError::internal(format!("cannot rasterize the page: {e}")))?,
        };
        Ok::<_, OpError>((bytes, rendered))
    })
    .await??;
    let report = resolve::report_at(entry.session(), ri)?;
    let page = report
        .pages
        .get(pi)
        .ok_or_else(|| OpError::not_found("no such page"))?;
    let (visuals, summary) = visuals_out(&rendered, req.visual.as_deref());
    let size = (req.format == RenderFormat::Png)
        .then(|| png_size(&bytes))
        .flatten();
    let mut out = RenderOut {
        ok: summary.errors == 0 && data.is_none_or(|d| d.failed == 0),
        report: report.name.clone(),
        page: RenderedPageInfo {
            index: pi + 1,
            name: page.name.clone(),
            display_name: page.display_name.clone(),
            width: page.width,
            height: page.height,
        },
        visual: req.visual,
        format: req.format,
        svg: None,
        png_base64: None,
        output_path: None,
        bytes: bytes.len(),
        pixel_width: size.map(|s| s.0),
        pixel_height: size.map(|s| s.1),
        summary,
        visuals,
        slicers,
        data,
        duration_ms: 0,
    };
    match output {
        Some(path) => {
            let overwrite = req.overwrite;
            let target = path.clone();
            tokio::task::spawn_blocking(move || write_bytes(&target, overwrite, &bytes)).await??;
            out.output_path = Some(path.to_string_lossy().into_owned());
        }
        None if bytes.len() > MAX_INLINE_BYTES => {
            return Err(OpError::invalid_params(format!(
                "the render is {} bytes, more than the inline limit of {MAX_INLINE_BYTES}; pass outputPath or a smaller scale",
                bytes.len()
            )));
        }
        None => match req.format {
            RenderFormat::Svg => {
                out.svg = Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            RenderFormat::Png => {
                out.png_base64 = Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
            }
        },
    }
    out.duration_ms = millis(started);
    Ok(out)
}

// ---------------------------------------------------------------------------

/// `render.page`.
#[derive(Debug)]
pub struct RenderPage;

impl Command for RenderPage {
    type Params = RenderPageParams;
    type Output = RenderOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "render.page",
            title: "Render a page",
            summary: "Renders a report page to SVG or PNG (inline, base64 for PNG, or to outputPath) with per-visual status; loads the data on first use.",
            category: Category::Render,
            long_running: true,
            writes_files: true,
            examples: &[
                ("Active page as SVG", r#"{"docId": "d1"}"#),
                (
                    "A page as PNG with a slicer selection",
                    r#"{"docId": "d1", "report": "Overview", "page": "1", "format": "png", "scale": 1, "slicers": [{"field": "Region", "values": ["West"]}]}"#,
                ),
                (
                    "To a file",
                    r#"{"docId": "d1", "report": "Stores", "format": "png", "scale": 2, "outputPath": "target/tmp/stores.png", "overwrite": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        render(
            ws,
            entry,
            Request {
                report: p.report,
                page: p.page,
                visual: None,
                format: p.format,
                scale: p.scale,
                slicers: p.slicers,
                output_path: p.output_path,
                overwrite: p.overwrite,
            },
            ctx,
        )
        .await
    }
}

/// `render.visual`.
#[derive(Debug)]
pub struct RenderVisual;

impl Command for RenderVisual {
    type Params = RenderVisualParams;
    type Output = RenderOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "render.visual",
            title: "Render a visual",
            summary: "Renders one visual of a page, cropped to its container, to SVG or PNG (with the page's filters, slicers and interactions).",
            category: Category::Render,
            long_running: true,
            writes_files: true,
            examples: &[(
                "One visual as PNG",
                r#"{"docId": "d1", "page": "1", "visual": "f7a9c9413fef68532313", "format": "png", "scale": 2}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        render(
            ws,
            entry,
            Request {
                report: p.report,
                page: p.page,
                visual: Some(p.visual),
                format: p.format,
                scale: p.scale,
                slicers: p.slicers,
                output_path: p.output_path,
                overwrite: p.overwrite,
            },
            ctx,
        )
        .await
    }
}
