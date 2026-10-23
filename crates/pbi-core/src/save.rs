//! Writers for opened documents: a PBIP project ([`save_as_pbip`]) and a
//! PBIT template ([`export_pbit`]).
//!
//! # PBIP
//!
//! ```text
//! <out>/<Project>.pbip
//! <out>/.gitignore
//! <out>/<Model>.SemanticModel/{definition.pbism, .platform, diagramLayout.json, definition/**/*.tmdl}
//! <out>/<Report>.Report/{definition.pbir, .platform, definition/**, StaticResources/**}
//! ```
//!
//! - The model is written as TMDL: with
//!   [`pbi_tmdl::write_model_preserving`] when it was read from TMDL (unedited
//!   files stay byte-identical), canonically otherwise (TMSL, VertiPaq
//!   metadata; `vertipaq.*` storage facts are dropped).
//! - PBIR reports are written with [`pbi_pbir::write_pbir`] (unedited files
//!   byte-identical). Legacy layouts (PBIX `Report/Layout`, PBIR-Legacy) are
//!   written as a PBIR-Legacy `report.json`; conversion to PBIR is not
//!   implemented.
//! - Folder names, `.pbip`, `.platform`, `definition.pbism` and
//!   `diagramLayout.json` are kept from a PBIP source; otherwise they are
//!   generated (`definition.pbir` binds reports to the model `byPath`, or
//!   `byConnection` from the PBIX `Connections` of a live report).
//! - Data caches (`.pbi/cache.abf`) and local settings are not written.
//!
//! # PBIT
//!
//! A ZIP with `Version`, `[Content_Types].xml`, `DataModelSchema` (TMSL,
//! UTF-16LE), the first report (`Report/definition/**` + `StaticResources`
//! for PBIR, `Report/Layout` for legacy layouts), `DiagramLayout` (when the
//! source had one), `Settings` and `Metadata`.

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Component, Path};
use std::sync::Arc;

use pbi_model::{DatasetReference, Model, SourceFormat};
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use crate::container::Container;
use crate::error::CoreError;
use crate::reader::{
    ModelSource, OpenedDocument, ReportSource, file_name, file_stem, item_name, join,
};

/// Options for [`save_as_pbip`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveOptions {
    /// Write into a folder that is not empty (files with the same paths are
    /// replaced, others are left alone). Default `false`.
    pub overwrite: bool,
}

const DEFAULT_PBISM: &str = "{\n  \"version\": \"4.2\",\n  \"settings\": {}\n}";
const PLATFORM_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/gitIntegration/platformProperties/2.0.0/schema.json";
const PBIR_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json";
const GITIGNORE: &str = "**/.pbi/localSettings.json\n**/.pbi/cache.abf\n";
const DEFAULT_VERSION: &str = "1.32";

/// Writes `doc` as a PBIP project into `out_dir` (created if missing).
///
/// # Errors
/// [`CoreError::Save`] if `out_dir` is not empty (without
/// [`SaveOptions::overwrite`]) or the document cannot be represented (e.g.
/// a report with no model to bind to), and [`CoreError::Io`] on write
/// failures.
pub fn save_as_pbip(
    doc: &OpenedDocument,
    out_dir: &Path,
    opts: &SaveOptions,
) -> Result<(), CoreError> {
    let files = pbip_files(doc)?;
    if out_dir.exists() {
        let mut entries = fs::read_dir(out_dir).map_err(|e| CoreError::io(out_dir, e))?;
        if !opts.overwrite && entries.next().is_some() {
            return Err(CoreError::Save(format!(
                "{} is not empty (set SaveOptions::overwrite to write into it)",
                out_dir.display()
            )));
        }
    }
    fs::create_dir_all(out_dir).map_err(|e| CoreError::io(out_dir, e))?;
    for (rel, bytes) in files {
        let rel_path = Path::new(&rel);
        if rel.is_empty()
            || !rel_path
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(CoreError::Save(format!(
                "refusing to write unsafe path {rel:?}"
            )));
        }
        let path = out_dir.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
        }
        fs::write(&path, bytes).map_err(|e| CoreError::io(&path, e))?;
    }
    Ok(())
}

/// The files [`save_as_pbip`] writes, as `(relative path, bytes)` pairs.
///
/// # Errors
/// As [`save_as_pbip`], without the I/O errors.
pub fn pbip_files(doc: &OpenedDocument) -> Result<Vec<(String, Vec<u8>)>, CoreError> {
    let mut out = Files::default();
    let project = project_name(doc);
    let pbip_source = doc.document.source.format == SourceFormat::Pbip;

    // Semantic models.
    let mut model_folder = None;
    if let (Some(model), Some(source)) = (&doc.document.model, &doc.model_source) {
        let folder = out.unique(model_folder_name(source, &project));
        write_model_folder(&mut out, &folder, model, source)?;
        model_folder = Some(folder);
    } else if let Some(model) = &doc.document.model {
        let folder = out.unique(format!("{}.SemanticModel", safe_name(&project)));
        write_model_tmdl(&mut out, &folder, model, None);
        out.put(
            join(&folder, "definition.pbism"),
            DEFAULT_PBISM.as_bytes().to_vec(),
        );
        out.put(
            join(&folder, ".platform"),
            platform("SemanticModel", &project),
        );
        model_folder = Some(folder);
    }
    let mut model_folders: Vec<String> = model_folder.iter().cloned().collect();
    for extra in &doc.extra_models {
        let folder = out.unique(model_folder_name(&extra.source, &project));
        write_model_folder(&mut out, &folder, &extra.model, &extra.source)?;
        model_folders.push(folder);
    }

    // Reports.
    let mut report_folders = Vec::new();
    for (i, source) in doc.report_sources.iter().enumerate() {
        let report = doc.document.reports.get(i);
        let name = report.map_or_else(|| project.clone(), |r| r.name.clone());
        let folder = out.unique(report_folder_name(doc, source, &name));
        let binding = || {
            definition_pbir(
                doc,
                report.map(|r| &r.dataset_reference),
                model_folder.as_deref(),
            )
        };
        match source {
            ReportSource::Pbir { report: pbir, .. } => {
                let rebound = rebind(pbir, &model_folders, model_folder.as_deref());
                let pbir = rebound.as_ref().unwrap_or(pbir);
                let mut has_definition = false;
                let mut has_platform = false;
                for (path, bytes) in pbi_pbir::write_pbir(pbir) {
                    if matches!(path.as_str(), "LinguisticSchema" | "Layout")
                        || path.starts_with(".pbi/")
                    {
                        continue;
                    }
                    has_definition |= path == "definition.pbir";
                    has_platform |= path == ".platform";
                    out.put(join(&folder, &path), bytes);
                }
                if !has_definition {
                    out.put(join(&folder, "definition.pbir"), binding()?);
                }
                if !has_platform {
                    out.put(join(&folder, ".platform"), platform("Report", &name));
                }
            }
            ReportSource::Legacy {
                folder: src_folder,
                report: legacy,
                ..
            } => {
                let layout = serde_json::to_vec(&legacy.layout)
                    .map_err(|e| CoreError::Save(format!("report `{name}`: {e}")))?;
                out.put(join(&folder, "report.json"), layout);
                let original = |rel: &str| doc.container.read(&join(src_folder, rel)).ok();
                let definition = match original("definition.pbir") {
                    Some(bytes) => bytes,
                    None => binding()?,
                };
                out.put(join(&folder, "definition.pbir"), definition);
                let platform_bytes =
                    original(".platform").unwrap_or_else(|| platform("Report", &name));
                out.put(join(&folder, ".platform"), platform_bytes);
                copy_tree(
                    &mut out,
                    doc.container.as_ref(),
                    &join(src_folder, "StaticResources"),
                    &join(&folder, "StaticResources"),
                );
            }
        }
        report_folders.push(folder);
    }

    // Project files.
    let pbip_name = format!("{}.pbip", safe_name(&project));
    let original_pbip = doc
        .detection
        .parts
        .pbip_file
        .as_deref()
        .filter(|_| pbip_source)
        .and_then(|p| doc.container.read(p).ok());
    let pbip = match original_pbip {
        Some(bytes) => bytes,
        None => {
            let artifacts: Vec<Value> = report_folders
                .first()
                .map(|f| json!({ "report": { "path": f } }))
                .into_iter()
                .collect();
            pretty(&json!({
                "version": "1.0",
                "artifacts": artifacts,
                "settings": { "enableAutoRecovery": true }
            }))
        }
    };
    out.put(pbip_name, pbip);
    let gitignore = if pbip_source {
        doc.container.read(".gitignore").ok()
    } else {
        None
    };
    out.put(
        ".gitignore".to_owned(),
        gitignore.unwrap_or_else(|| GITIGNORE.as_bytes().to_vec()),
    );
    Ok(out.files)
}

/// Writes `doc` as a PBIT template to `out_path`.
///
/// # Errors
/// [`CoreError::Save`] if the document has no local model or no report, and
/// [`CoreError::Io`] on write failures.
pub fn export_pbit(doc: &OpenedDocument, out_path: &Path) -> Result<(), CoreError> {
    let bytes = pbit_bytes(doc)?;
    fs::write(out_path, bytes).map_err(|e| CoreError::io(out_path, e))
}

/// The PBIT archive [`export_pbit`] writes.
///
/// # Errors
/// As [`export_pbit`], without the I/O errors.
pub fn pbit_bytes(doc: &OpenedDocument) -> Result<Vec<u8>, CoreError> {
    let model = doc
        .document
        .model
        .as_ref()
        .ok_or_else(|| CoreError::Save("a PBIT needs a local semantic model".into()))?;
    let source = doc
        .report_sources
        .first()
        .ok_or_else(|| CoreError::Save("a PBIT needs a report".into()))?;
    let package = matches!(
        doc.document.source.format,
        SourceFormat::Pbix | SourceFormat::Pbit
    );
    let original = |name: &str| {
        if package {
            doc.container.read(name).ok()
        } else {
            None
        }
    };

    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    parts.push((
        "Version".into(),
        original("Version").unwrap_or_else(|| utf16le(DEFAULT_VERSION)),
    ));

    let mut format = match &doc.model_source {
        Some(ModelSource::Tmsl { write_options, .. }) => write_options.format.clone(),
        _ => pbi_tmsl::TmslFormat::data_model_schema(),
    };
    format.encoding = pbi_tmsl::TextEncoding::Utf16Le;
    format.bom = false;
    let schema = pbi_tmsl::write_bytes(
        &crate::reader::model_for_writing(model),
        &pbi_tmsl::WriteOptions {
            format,
            ..pbi_tmsl::WriteOptions::default()
        },
    );
    parts.push(("DataModelSchema".into(), schema));
    if let Some(diagram) = original("DiagramLayout") {
        parts.push(("DiagramLayout".into(), diagram));
    }

    match source {
        ReportSource::Pbir { report, .. } => {
            for (path, bytes) in pbi_pbir::write_pbir(report) {
                if matches!(path.as_str(), "definition.pbir" | ".platform")
                    || path.starts_with(".pbi/")
                {
                    continue;
                }
                parts.push((format!("Report/{path}"), bytes));
            }
        }
        ReportSource::Legacy {
            folder,
            report: legacy,
            ..
        } => {
            let layout = serde_json::to_string(&legacy.layout)
                .map_err(|e| CoreError::Save(format!("report layout: {e}")))?;
            parts.push(("Report/Layout".into(), utf16le(&layout)));
            let mut files = Files::default();
            copy_tree(
                &mut files,
                doc.container.as_ref(),
                &join(folder, "StaticResources"),
                "Report/StaticResources",
            );
            parts.extend(files.files);
        }
    }

    parts.push((
        "Settings".into(),
        original("Settings").unwrap_or_else(|| {
            utf16le(r#"{"Version":4,"ReportSettings":{},"QueriesSettings":{"TypeDetectionEnabled":true,"RelationshipImportEnabled":true}}"#)
        }),
    ));
    parts.push((
        "Metadata".into(),
        original("Metadata").unwrap_or_else(|| {
            utf16le(r#"{"Version":5,"AutoCreatedRelationships":[],"FileDescription":""}"#)
        }),
    ));
    parts.insert(1, ("[Content_Types].xml".into(), content_types(&parts)));

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let zip_err = |e: zip::result::ZipError| CoreError::Save(format!("PBIT archive: {e}"));
    for (name, bytes) in &parts {
        zip.start_file(name.as_str(), options).map_err(zip_err)?;
        zip.write_all(bytes)
            .map_err(|e| CoreError::Save(format!("PBIT archive: {e}")))?;
    }
    Ok(zip.finish().map_err(zip_err)?.into_inner())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Output files with unique folder names.
#[derive(Default)]
struct Files {
    files: Vec<(String, Vec<u8>)>,
    folders: HashSet<String>,
}

impl Files {
    fn put(&mut self, path: String, bytes: Vec<u8>) {
        if let Some(slot) = self.files.iter_mut().find(|(p, _)| *p == path) {
            slot.1 = bytes;
        } else {
            self.files.push((path, bytes));
        }
    }

    /// `name`, or `name` with a ` (n)` suffix before the item extension
    /// when it is taken (case-insensitive).
    fn unique(&mut self, name: String) -> String {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let mut candidate = name;
        let mut n = 2;
        while !self.folders.insert(candidate.to_lowercase()) {
            candidate = format!("{stem} ({n}){ext}");
            n += 1;
        }
        candidate
    }
}

fn pretty(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).unwrap_or_default()
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A file-system-safe single path component.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_start_matches('.');
    if trimmed.is_empty() {
        "Untitled".to_owned()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Last component of the root of `container` (folder name on disk).
fn root_name(container: &dyn Container) -> String {
    let label = container.root_label().replace('\\', "/");
    file_name(label.trim_end_matches('/')).to_owned()
}

fn has_item_suffix(name: &str, suffixes: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    suffixes
        .iter()
        .any(|s| lower.ends_with(&s.to_ascii_lowercase()) && lower.len() > s.len())
}

fn project_name(doc: &OpenedDocument) -> String {
    if let Some(pbip) = &doc.detection.parts.pbip_file {
        return file_stem(file_name(pbip)).to_owned();
    }
    let source = doc.document.source.path.replace('\\', "/");
    let name = file_name(source.trim_end_matches('/'));
    let name = if doc.container.kind() == crate::container::ContainerKind::Directory {
        name
    } else {
        file_stem(name)
    };
    let name = item_name(name);
    if name.is_empty() || name == "." {
        doc.document
            .reports
            .first()
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "Project".into())
    } else {
        name
    }
}

fn model_folder_name(source: &ModelSource, project: &str) -> String {
    let folder = source.folder();
    let candidate = if folder.is_empty() {
        root_name(source.container().as_ref())
    } else {
        file_name(folder).to_owned()
    };
    if has_item_suffix(&candidate, &[".SemanticModel", ".Dataset"]) {
        safe_name(&candidate)
    } else {
        format!("{}.SemanticModel", safe_name(project))
    }
}

fn report_folder_name(doc: &OpenedDocument, source: &ReportSource, name: &str) -> String {
    let folder = source.folder();
    let candidate = if folder.is_empty() {
        root_name(doc.container.as_ref())
    } else {
        file_name(folder).to_owned()
    };
    if has_item_suffix(&candidate, &[".Report"]) {
        safe_name(&candidate)
    } else {
        format!("{}.Report", safe_name(name))
    }
}

fn platform(item_type: &str, display_name: &str) -> Vec<u8> {
    pretty(&json!({
        "$schema": PLATFORM_SCHEMA,
        "metadata": { "type": item_type, "displayName": display_name },
        "config": { "version": "2.0", "logicalId": pseudo_uuid(item_type, display_name) },
    }))
}

/// A random-looking v4-style UUID (not cryptographic; `logicalId`s only
/// need to be unique per workspace).
fn pseudo_uuid(a: &str, b: &str) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let state = RandomState::new();
    let mut words = [0u64; 2];
    for (i, w) in words.iter_mut().enumerate() {
        let mut h = state.build_hasher();
        h.write(a.as_bytes());
        h.write(b.as_bytes());
        h.write_usize(i);
        if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            h.write_u128(t.as_nanos());
        }
        *w = h.finish();
    }
    let hi = words[0];
    let lo = words[1];
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        hi >> 32,
        (hi >> 16) & 0xffff,
        hi & 0x0fff,
        ((lo >> 48) & 0x3fff) | 0x8000,
        lo & 0xffff_ffff_ffff
    )
}

/// A copy of `pbir` whose `byPath` points at `primary` when it names none
/// of the saved model folders (e.g. a model folder renamed on save).
fn rebind(
    pbir: &pbi_pbir::PbirReport,
    saved: &[String],
    primary: Option<&str>,
) -> Option<pbi_pbir::PbirReport> {
    let primary = primary?;
    let path = pbir.definition_pbir.as_ref()?.by_path()?;
    let target = file_name(path.replace('\\', "/").trim_end_matches('/')).to_owned();
    if saved.iter().any(|f| f.eq_ignore_ascii_case(&target)) {
        return None;
    }
    let mut copy = pbir.clone();
    let by_path = copy
        .definition_pbir
        .as_mut()?
        .dataset_reference
        .as_mut()?
        .by_path
        .as_mut()?;
    by_path.path = Some(format!("../{primary}"));
    Some(copy)
}

/// `definition.pbir` for a report that has none (PBIX/PBIT sources).
fn definition_pbir(
    doc: &OpenedDocument,
    reference: Option<&DatasetReference>,
    model_folder: Option<&str>,
) -> Result<Vec<u8>, CoreError> {
    let dataset = if let Some(folder) = model_folder {
        json!({ "byPath": { "path": format!("../{folder}") } })
    } else if let Some(DatasetReference::ByConnection { .. }) = reference {
        let conn = live_connection_raw(doc).ok_or_else(|| {
            CoreError::Save("the live connection of the report is not available".into())
        })?;
        json!({ "byConnection": conn })
    } else {
        return Err(CoreError::Save(
            "the report has no local model and no live connection to bind to".into(),
        ));
    };
    Ok(pretty(&json!({
        "$schema": PBIR_SCHEMA,
        "version": "4.0",
        "datasetReference": dataset,
    })))
}

/// `byConnection` from the original (unredacted) PBIX `Connections` part,
/// in `definition.pbir` key style.
fn live_connection_raw(doc: &OpenedDocument) -> Option<Value> {
    let bytes = doc.container.read("Connections").ok()?;
    let text = crate::container::decode_text(&bytes).text;
    let json: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let conn = json.get("Connections")?.as_array()?.first()?;
    let mut out = serde_json::Map::new();
    for (from, to) in [
        ("ConnectionString", "connectionString"),
        ("PbiServiceModelId", "pbiServiceModelId"),
        ("PbiModelVirtualServerName", "pbiModelVirtualServerName"),
        ("PbiModelDatabaseName", "pbiModelDatabaseName"),
        ("Name", "name"),
        ("ConnectionType", "connectionType"),
    ] {
        if let Some(v) = conn.get(from) {
            out.insert(to.to_owned(), v.clone());
        }
    }
    Some(Value::Object(out))
}

fn write_model_folder(
    out: &mut Files,
    folder: &str,
    model: &Model,
    source: &ModelSource,
) -> Result<(), CoreError> {
    let src: &Arc<dyn Container> = source.container();
    let src_folder = source.folder();
    let original = |rel: &str| src.read(&join(src_folder, rel)).ok();
    let previous = match source {
        ModelSource::Tmdl { files, .. } => Some(files.as_slice()),
        _ => None,
    };
    if matches!(source, ModelSource::Tmsl { .. }) {
        let mut model = model.clone();
        crate::reader::strip_tmsl_shadows(&mut model);
        write_model_tmdl(out, folder, &model, previous);
    } else {
        write_model_tmdl(out, folder, model, previous);
    }
    let from_folder = !matches!(source, ModelSource::Abf { .. });
    // A TMSL-era `definition.pbism` (version 1.x) does not allow TMDL.
    let pbism = match (previous.is_some(), original("definition.pbism")) {
        (true, Some(bytes)) => bytes,
        _ => DEFAULT_PBISM.as_bytes().to_vec(),
    };
    out.put(join(folder, "definition.pbism"), pbism);
    let platform_bytes = original(".platform")
        .filter(|_| from_folder)
        .unwrap_or_else(|| platform("SemanticModel", &item_name(folder)));
    out.put(join(folder, ".platform"), platform_bytes);
    if from_folder && let Some(diagram) = original("diagramLayout.json") {
        out.put(join(folder, "diagramLayout.json"), diagram);
    }
    Ok(())
}

fn write_model_tmdl(
    out: &mut Files,
    folder: &str,
    model: &Model,
    previous: Option<&[pbi_tmdl::TmdlFile]>,
) {
    let model = crate::reader::model_for_writing(model);
    let files = match previous {
        Some(prev) => pbi_tmdl::write_model_preserving(&model, None, prev),
        None => pbi_tmdl::write_model(&model, None),
    };
    for (path, text) in files {
        out.put(
            join(folder, &format!("definition/{path}")),
            text.into_bytes(),
        );
    }
}

/// Copies every file under `from` in `container` to `to`.
fn copy_tree(out: &mut Files, container: &dyn Container, from: &str, to: &str) {
    let cut = if from.is_empty() { 0 } else { from.len() + 1 };
    for e in container.list_prefix(from) {
        if e.is_dir {
            continue;
        }
        if let (Some(rel), Ok(bytes)) = (e.path.get(cut..), container.read(&e.path)) {
            out.put(join(to, rel), bytes);
        }
    }
}

/// `[Content_Types].xml` for the parts of a PBIT.
fn content_types(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut xml = String::from(
        "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"json\" ContentType=\"\" />",
    );
    for (name, _) in parts {
        if name.to_ascii_lowercase().ends_with(".json") {
            continue;
        }
        let escaped = name
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        xml.push_str(&format!(
            "<Override PartName=\"/{escaped}\" ContentType=\"\" />"
        ));
    }
    xml.push_str("</Types>");
    xml.into_bytes()
}
