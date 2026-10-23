//! PBIP projects and their item folders: `.pbip` project folders (and
//! Fabric Git / deployment exports without a `.pbip`), `*.SemanticModel`
//! folders, `*.Report` folders and loose TMDL folders.

use std::collections::HashMap;
use std::sync::Arc;

use pbi_model::{DatasetReference, Diagnostic, Origin, SourceFormat};

use super::files::{item_name, join};
use super::model::{Loaded, check_cache_abf, load_model_folder, load_tmdl};
use super::report::{LoadedReport, load_legacy, load_pbir};
use super::{Builder, ExtraModel, codes};
use crate::container::{Container, DirContainer};
use crate::detect::{
    self, DetectedFormat, ModelFlavour, ReportFlavour, ReportPart, SemanticModelPart,
    detect_container,
};

/// JSON metadata files of a semantic model folder.
const MODEL_PARTS: &[(&str, &str)] = &[
    ("definition.pbism", "pbism"),
    ("definition.pbidataset", "pbism"),
    (".platform", "platform"),
    (".pbi/localSettings.json", "localSettings"),
    (".pbi/editorSettings.json", "editorSettings"),
    ("diagramLayout.json", "diagramLayout"),
];

/// JSON metadata files of a report folder.
const REPORT_PARTS: &[(&str, &str)] = &[
    (".platform", "platform"),
    (".pbi/localSettings.json", "localSettings"),
];

struct ModelEntry {
    folder: String,
    loaded: Loaded,
}

struct ReportEntry {
    part: ReportPart,
    loaded: LoadedReport,
}

pub(crate) fn read(b: &mut Builder<'_>) {
    let format = b.detection.format.clone();
    if format == DetectedFormat::TmdlFolder {
        b.doc.source.format = SourceFormat::TmdlFolder;
        let container = Arc::clone(&b.container);
        let loaded = load_tmdl(&container, "", "", &mut b.doc.diagnostics);
        if let Some(loaded) = loaded {
            b.consumed.extend(loaded.consumed);
            b.doc.model = Some(loaded.model);
            b.model_source = Some(loaded.source);
        }
        return;
    }

    if let Some(pbip) = b.detection.parts.pbip_file.clone() {
        b.json_part(&pbip, "pbip");
    }

    let mut models = Vec::new();
    for part in b.detection.parts.semantic_models.clone() {
        if let Some(entry) = read_model_folder(b, &part) {
            models.push(entry);
        }
    }

    let mut reports = Vec::new();
    for part in ordered_reports(b) {
        for (rel, kind) in REPORT_PARTS {
            b.json_part(&join(&part.path, rel), kind);
        }
        let container = Arc::clone(&b.container);
        let loaded = match part.flavour {
            ReportFlavour::Pbir => {
                load_pbir(container.as_ref(), &part.path, &mut b.doc.diagnostics)
            }
            ReportFlavour::PbirLegacy => load_legacy(
                container.as_ref(),
                &part.path,
                &join(&part.path, "report.json"),
                &mut b.doc.diagnostics,
            ),
            ReportFlavour::LegacyLayout | ReportFlavour::Unknown => {
                b.diag(
                    Diagnostic::warning(
                        codes::REPORT_FAILED,
                        format!(
                            "report folder `{}` has neither `definition/` (PBIR) nor `report.json` (PBIR-Legacy)",
                            display(&part.path)
                        ),
                    )
                    .with_origin(Origin::file(part.path.clone())),
                );
                None
            }
        };
        if let Some(mut loaded) = loaded {
            if loaded.report.name.is_empty() {
                loaded.report.name = if part.path.is_empty() {
                    item_name(&b.name)
                } else {
                    item_name(&part.path)
                };
            }
            b.consumed.extend(loaded.consumed.drain(..));
            reports.push(ReportEntry { part, loaded });
        }
    }

    // A lone report folder: its byPath model usually sits next to it.
    if models.is_empty()
        && let [only] = reports.as_slice()
        && only.part.path.is_empty()
        && let Some(entry) = external_model(b, &only.part)
    {
        models.push(entry);
    }

    b.doc.source.format = match &format {
        DetectedFormat::PbipReportFolder
            if reports
                .first()
                .is_some_and(|r| r.part.flavour == ReportFlavour::PbirLegacy) =>
        {
            SourceFormat::PbirLegacy
        }
        other => super::source_format_of(other),
    };

    bind(b, models, reports);
}

fn display(folder: &str) -> &str {
    if folder.is_empty() { "." } else { folder }
}

fn read_model_folder(b: &mut Builder<'_>, part: &SemanticModelPart) -> Option<ModelEntry> {
    for (rel, kind) in MODEL_PARTS {
        b.json_part(&join(&part.path, rel), kind);
    }
    let container = Arc::clone(&b.container);
    let loaded = load_model_folder(&container, &part.path, part.flavour, &mut b.doc.diagnostics);
    if let Some(cache) = check_cache_abf(
        &container,
        &part.path,
        loaded.as_ref().map(|l| &l.model),
        b.opts,
        &mut b.doc.diagnostics,
    ) {
        b.parts.push(cache);
    }
    let mut loaded = loaded?;
    b.consumed.extend(loaded.consumed.drain(..));
    Some(ModelEntry {
        folder: part.path.clone(),
        loaded,
    })
}

/// Reports in `.pbip` artifact order first, then the others by folder.
fn ordered_reports(b: &Builder<'_>) -> Vec<ReportPart> {
    let parts = &b.detection.parts;
    let mut rank: HashMap<&str, usize> = HashMap::new();
    for (i, a) in parts.pbip_artifacts.iter().enumerate() {
        let key = a.trim_end_matches(['/', '\\']);
        rank.entry(key).or_insert(i);
    }
    let mut reports = parts.reports.clone();
    reports.sort_by_key(|r| {
        (
            rank.get(r.path.as_str()).copied().unwrap_or(usize::MAX),
            r.path.clone(),
        )
    });
    reports
}

/// Resolves a lone report folder's `byPath` model on disk, next to the
/// opened folder. Only folders that look like semantic models are read.
fn external_model(b: &mut Builder<'_>, part: &ReportPart) -> Option<ModelEntry> {
    let detect::DatasetReference::ByPath {
        path,
        resolved: None,
    } = &part.dataset
    else {
        return None;
    };
    let base = b.base_dir.clone()?;
    let rel = path.replace('\\', "/");
    if rel.starts_with('/')
        || rel.contains(':')
        || rel.split('/').filter(|s| *s == "..").count() > 2
    {
        return None;
    }
    let target = base.join(&rel);
    let target = std::fs::canonicalize(&target).ok()?;
    if !target.is_dir() {
        return None;
    }
    let dir = match DirContainer::open(&target, b.opts.limits) {
        Ok(d) => d,
        Err(e) => {
            b.diag(
                Diagnostic::warning(
                    codes::MODEL_NOT_FOUND,
                    format!("semantic model `{path}` could not be opened: {e}"),
                )
                .with_origin(Origin::file("definition.pbir")),
            );
            return None;
        }
    };
    let detection = detect_container(&dir, target.file_name().and_then(|n| n.to_str()));
    if detection.format != DetectedFormat::PbipSemanticModelFolder {
        return None;
    }
    let flavour = detection
        .parts
        .semantic_models
        .first()
        .map_or(ModelFlavour::Unknown, |m| m.flavour);
    let container: Arc<dyn Container> = Arc::new(dir);
    let mut diags = Vec::new();
    let loaded = load_model_folder(&container, "", flavour, &mut diags);
    if let Some(mut cache) = check_cache_abf(
        &container,
        "",
        loaded.as_ref().map(|l| &l.model),
        b.opts,
        &mut diags,
    ) {
        cache.path = join(&rel, &cache.path);
        b.parts.push(cache);
    }
    // Origins of the external model are relative to the report folder.
    for d in &mut diags {
        if let Some(o) = &mut d.origin {
            o.path = join(&rel, &o.path);
        }
    }
    b.doc.diagnostics.extend(diags);
    let loaded = loaded?;
    b.diag(
        Diagnostic::info(
            codes::EXTERNAL_MODEL,
            format!("semantic model loaded from `{rel}` (the report's byPath, next to the opened folder)"),
        )
        .with_origin(Origin::file("definition.pbir")),
    );
    Some(ModelEntry {
        folder: rel,
        loaded,
    })
}

/// Picks the document model, binds reports to models and records the
/// extra models.
fn bind(b: &mut Builder<'_>, models: Vec<ModelEntry>, reports: Vec<ReportEntry>) {
    let target_of = |r: &ReportEntry| -> Option<String> {
        match &r.part.dataset {
            detect::DatasetReference::ByPath {
                resolved: Some(m), ..
            } => Some(m.clone()),
            detect::DatasetReference::ByPath {
                path,
                resolved: None,
            } => Some(path.replace('\\', "/")),
            _ => None,
        }
    };
    let mut uses: Vec<usize> = vec![0; models.len()];
    for r in &reports {
        if let Some(t) = target_of(r)
            && let Some(i) = models.iter().position(|m| m.folder == t)
        {
            uses[i] += 1;
        }
    }
    let primary = (0..models.len()).max_by(|a, b| uses[*a].cmp(&uses[*b]).then(b.cmp(a)));
    let primary_folder = primary
        .and_then(|i| models.get(i))
        .map(|m| m.folder.clone());
    if models.len() > 1
        && let Some(folder) = &primary_folder
    {
        let others: Vec<&str> = models
            .iter()
            .filter(|m| &m.folder != folder)
            .map(|m| m.folder.as_str())
            .collect();
        b.diag(Diagnostic::info(
            codes::MULTIPLE_MODELS,
            format!(
                "{} semantic models; `{folder}` is the document model, the others ({}) are kept as extra models",
                models.len(),
                others.join(", ")
            ),
        ));
    }

    let model_folders: Vec<String> = models.iter().map(|m| m.folder.clone()).collect();
    for (i, entry) in models.into_iter().enumerate() {
        if Some(i) == primary {
            b.doc.model = Some(entry.loaded.model);
            b.model_source = Some(entry.loaded.source);
        } else {
            b.extra_models.push(ExtraModel {
                model: entry.loaded.model,
                source: entry.loaded.source,
            });
        }
    }

    if reports.is_empty() && b.detection.format != DetectedFormat::PbipSemanticModelFolder {
        b.diag(Diagnostic::info(codes::NO_REPORT, "no report found"));
    }
    for r in reports {
        let folder = r.part.path.clone();
        let origin = Origin::file(join(&folder, "definition.pbir"));
        match (&r.loaded.report.dataset_reference, target_of(&r)) {
            (DatasetReference::ByPath { path }, Some(target)) => {
                if Some(&target) == primary_folder.as_ref() {
                    // Bound to the document model.
                } else if model_folders.contains(&target) {
                    b.diag(
                        Diagnostic::info(
                            codes::MULTIPLE_MODELS,
                            format!(
                                "report `{}` uses semantic model `{target}`, which is not the document model (see extra models)",
                                r.loaded.report.name
                            ),
                        )
                        .with_origin(origin),
                    );
                } else {
                    b.diag(
                        Diagnostic::warning(
                            codes::MODEL_NOT_FOUND,
                            format!(
                                "report `{}` references semantic model `{path}`, which was not found or could not be loaded",
                                r.loaded.report.name
                            ),
                        )
                        .with_origin(origin),
                    );
                }
            }
            (DatasetReference::ByConnection { .. }, _) => {
                b.diag(
                    Diagnostic::info(
                        codes::REMOTE_MODEL,
                        format!(
                            "report `{}` uses a service-hosted model (byConnection): {}",
                            r.loaded.report.name,
                            super::REMOTE_DESCRIPTION
                        ),
                    )
                    .with_origin(origin),
                );
            }
            (DatasetReference::None, _) => {
                b.diag(
                    Diagnostic::info(
                        codes::NO_MODEL,
                        format!(
                            "report `{}` has no dataset reference (definition.pbir missing or empty)",
                            r.loaded.report.name
                        ),
                    )
                    .with_origin(origin),
                );
            }
            _ => {}
        }
        b.doc.reports.push(r.loaded.report);
        b.report_sources.push(r.loaded.source);
    }
    if b.doc.model.is_none()
        && b.detection.format == DetectedFormat::PbipSemanticModelFolder
        && !b.doc.has_errors()
    {
        b.diag(Diagnostic::info(
            codes::NO_MODEL,
            "no semantic model loaded",
        ));
    }
}
