//! Semantic model loading: TMDL folders, TMSL JSON and Analysis Services
//! backups (PBIX `DataModel`, `.abf`, PBIP `.pbi/cache.abf`).

use std::collections::BTreeSet;
use std::sync::Arc;

use pbi_model::{Diagnostic, Model, Origin, PropValue};
use pbi_vertipaq::xpress9::DataModelFormat;
use pbi_vertipaq::{AbfArchive, AbfError, ModelOptions};
use serde_json::Value;

use super::files::join;
use super::{MetadataPart, ModelSource, OpenOptions, codes};
use crate::container::Container;
use crate::detect::ModelFlavour;

/// A loaded model and where it came from.
pub(crate) struct Loaded {
    pub model: Model,
    pub source: ModelSource,
    /// Entries interpreted as the model definition.
    pub consumed: Vec<String>,
}

/// Loads the model of a semantic model folder (`folder` in `c`).
pub(crate) fn load_model_folder(
    c: &Arc<dyn Container>,
    folder: &str,
    flavour: ModelFlavour,
    diags: &mut Vec<Diagnostic>,
) -> Option<Loaded> {
    match flavour {
        ModelFlavour::Tmdl => load_tmdl(c, folder, &join(folder, "definition"), diags),
        ModelFlavour::Bim => load_tmsl(c, folder, &join(folder, "model.bim"), diags),
        ModelFlavour::Unknown => {
            diags.push(
                Diagnostic::warning(
                    codes::NO_MODEL,
                    format!(
                        "semantic model folder `{}` has neither TMDL files under `definition/` nor a `model.bim`",
                        display(folder)
                    ),
                )
                .with_origin(Origin::file(folder)),
            );
            None
        }
    }
}

fn display(folder: &str) -> &str {
    if folder.is_empty() { "." } else { folder }
}

/// Loads every `.tmdl` file under `definition_dir`.
pub(crate) fn load_tmdl(
    c: &Arc<dyn Container>,
    folder: &str,
    definition_dir: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<Loaded> {
    let paths: Vec<(String, u64)> = c
        .list_prefix(definition_dir)
        .into_iter()
        .filter(|e| !e.is_dir && e.path.to_ascii_lowercase().ends_with(".tmdl"))
        .map(|e| (e.path.clone(), e.size))
        .collect();
    let fail = |diags: &mut Vec<Diagnostic>, origin: &str, msg: String| {
        diags.push(Diagnostic::error(codes::MODEL_FAILED, msg).with_origin(Origin::file(origin)));
        None
    };
    if paths.is_empty() {
        return fail(
            diags,
            definition_dir,
            format!("no .tmdl files under `{}`", display(definition_dir)),
        );
    }
    if paths.len() > pbi_tmdl::MAX_FILES {
        return fail(
            diags,
            definition_dir,
            format!(
                "{} .tmdl files exceed the limit of {}",
                paths.len(),
                pbi_tmdl::MAX_FILES
            ),
        );
    }
    let mut files = Vec::with_capacity(paths.len());
    for (path, size) in &paths {
        if *size > pbi_tmdl::MAX_FILE_BYTES {
            return fail(
                diags,
                path,
                format!(
                    "{path}: {size} bytes exceeds the TMDL file limit of {} bytes; model not loaded",
                    pbi_tmdl::MAX_FILE_BYTES
                ),
            );
        }
        let bytes = match c.read(path) {
            Ok(b) => b,
            Err(e) => return fail(diags, path, format!("{path}: {e}; model not loaded")),
        };
        match String::from_utf8(bytes) {
            Ok(text) => files.push((path.clone(), text)),
            Err(_) => {
                return fail(
                    diags,
                    path,
                    format!("{path}: not valid UTF-8 text; model not loaded"),
                );
            }
        }
    }
    match pbi_tmdl::load_model(&files) {
        Ok(loaded) => {
            diags.extend(loaded.diagnostics);
            Some(Loaded {
                model: loaded.model,
                source: ModelSource::Tmdl {
                    folder: folder.to_owned(),
                    container: Arc::clone(c),
                    definition_dir: definition_dir.to_owned(),
                    files: loaded.files,
                },
                consumed: paths.into_iter().map(|(p, _)| p).collect(),
            })
        }
        Err(e) => {
            let origin = match &e {
                pbi_tmdl::TmdlError::Syntax { path, line, .. } => {
                    Origin::file(path.clone()).with_line(*line)
                }
                pbi_tmdl::TmdlError::Io { path, .. }
                | pbi_tmdl::TmdlError::Encoding { path }
                | pbi_tmdl::TmdlError::Limit { path, .. } => Origin::file(path.clone()),
            };
            diags.push(
                Diagnostic::error(codes::MODEL_FAILED, format!("TMDL model not loaded: {e}"))
                    .with_origin(origin),
            );
            None
        }
    }
}

/// Loads a TMSL entry (`model.bim`, `DataModelSchema`).
pub(crate) fn load_tmsl(
    c: &Arc<dyn Container>,
    folder: &str,
    entry: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<Loaded> {
    let bytes = match c.read(entry) {
        Ok(b) => b,
        Err(e) => {
            diags.push(
                Diagnostic::error(codes::MODEL_FAILED, format!("{entry}: {e}"))
                    .with_origin(Origin::file(entry)),
            );
            return None;
        }
    };
    let options = pbi_tmsl::ReadOptions {
        source_path: Some(entry.to_owned()),
    };
    match pbi_tmsl::read_bytes(&bytes, &options) {
        Ok(loaded) => {
            let write_options = loaded.write_options();
            diags.extend(loaded.diagnostics);
            Some(Loaded {
                model: loaded.model,
                source: ModelSource::Tmsl {
                    folder: folder.to_owned(),
                    container: Arc::clone(c),
                    entry: entry.to_owned(),
                    write_options,
                },
                consumed: vec![entry.to_owned()],
            })
        }
        Err(e) => {
            diags.push(
                Diagnostic::error(
                    codes::MODEL_FAILED,
                    format!("TMSL model not loaded from {entry}: {e}"),
                )
                .with_origin(Origin::file(entry)),
            );
            None
        }
    }
}

fn vertipaq_limits(opts: &OpenOptions) -> pbi_vertipaq::Limits {
    let default = pbi_vertipaq::Limits::DEFAULT;
    default.with_max_output_bytes(default.max_output_bytes.min(opts.limits.max_total_size))
}

/// Opens a backup from shared bytes without copying uncompressed input.
fn open_archive(data: Arc<[u8]>, opts: &OpenOptions) -> Result<(AbfArchive, bool), AbfError> {
    match pbi_vertipaq::abf::sniff(&data) {
        Some(DataModelFormat::Uncompressed) => Ok((AbfArchive::parse(data)?, false)),
        Some(_) => Ok((
            pbi_vertipaq::open_pbix_datamodel(&data, &vertipaq_limits(opts))?,
            true,
        )),
        None => Err(AbfError::UnknownSignature),
    }
}

/// Reads the model schema of a backup entry (`DataModel`, `.abf`).
/// `data` may hold the entry's bytes already (single-file input).
pub(crate) fn load_abf(
    c: &Arc<dyn Container>,
    entry: &str,
    data: Option<Arc<[u8]>>,
    opts: &OpenOptions,
    diags: &mut Vec<Diagnostic>,
) -> Option<Loaded> {
    let fail = |diags: &mut Vec<Diagnostic>, msg: String| {
        diags.push(Diagnostic::error(codes::MODEL_FAILED, msg).with_origin(Origin::file(entry)));
        None
    };
    let data = match data {
        Some(d) => d,
        None => match c.read(entry) {
            Ok(b) => Arc::from(b),
            Err(e) => return fail(diags, format!("{entry}: {e}")),
        },
    };
    let (archive, compressed) = match open_archive(data, opts) {
        Ok(a) => a,
        Err(e) => return fail(diags, format!("{entry}: cannot open the backup: {e}")),
    };
    let options = ModelOptions::default()
        .with_system_tables(opts.include_system_tables)
        .with_origin_path(Some(entry.to_owned()));
    match pbi_vertipaq::load_model(&archive, &options) {
        Ok(loaded) => {
            diags.extend(loaded.diagnostics.iter().cloned().map(|mut d| {
                if d.origin.is_none() {
                    d.origin = Some(Origin::file(entry));
                }
                d
            }));
            let folder = match entry.rsplit_once('/') {
                Some((dir, _)) => dir.to_owned(),
                None => String::new(),
            };
            Some(Loaded {
                model: loaded.model,
                source: ModelSource::Abf {
                    folder,
                    container: Arc::clone(c),
                    entry: entry.to_owned(),
                    compressed,
                    archive,
                    storage: loaded.storage,
                    schema_version: loaded.schema_version,
                },
                consumed: Vec::new(),
            })
        }
        Err(e) => fail(diags, format!("{entry}: cannot read the model schema: {e}")),
    }
}

/// User-visible table names of a model (no internal or auto date tables).
fn visible_tables(model: &Model) -> BTreeSet<String> {
    model
        .tables
        .iter()
        .filter(|t| {
            !t.extra
                .keys()
                .any(|k| k == "vertipaq.internalTable" || k == "vertipaq.autoDateTable")
                && !t.name.starts_with("LocalDateTable_")
                && !t.name.starts_with("DateTableTemplate_")
        })
        .map(|t| t.name.to_lowercase())
        .collect()
}

/// Notes the PBIP `.pbi/cache.abf` of a semantic model folder and, if
/// enabled, compares its schema with the loaded model. Returns a
/// `cacheAbf` metadata part with the cache's schema summary (`size`,
/// `xpress9`, `perFileCompression`, `metadataFormat`, `schemaVersion`,
/// `compatibilityLevel`, `tables`, `matchesDefinition`), or `None` if there
/// is no cache or it was not read.
pub(crate) fn check_cache_abf(
    c: &Arc<dyn Container>,
    folder: &str,
    model: Option<&Model>,
    opts: &OpenOptions,
    diags: &mut Vec<Diagnostic>,
) -> Option<MetadataPart> {
    let entry = join(folder, ".pbi/cache.abf");
    let info = c.entry(&entry).filter(|e| !e.is_dir)?;
    let origin = Origin::file(entry.clone());
    if !opts.load_pbix_schema {
        diags.push(
            Diagnostic::info(
                codes::CACHE_ABF,
                format!(
                    "local data cache {entry} ({} bytes) present; not read (schema loading disabled)",
                    info.size
                ),
            )
            .with_origin(origin),
        );
        return None;
    }
    let mut scratch = Vec::new();
    let Some(cached) = load_abf(c, &entry, None, opts, &mut scratch) else {
        let why = scratch
            .first()
            .map_or_else(String::new, |d| format!(": {}", d.message));
        diags.push(
            Diagnostic::warning(
                codes::CACHE_ABF,
                format!("local data cache {entry} could not be read{why}"),
            )
            .with_origin(origin),
        );
        return None;
    };
    let cache_tables = visible_tables(&cached.model);
    let matches = model.map(|m| visible_tables(m) == cache_tables);
    let part = cache_part(&entry, info.size, &cached, &cache_tables, matches);
    let message = match model {
        None => format!(
            "local data cache {entry} holds {} table(s); the model definition was not loaded",
            cache_tables.len()
        ),
        Some(model) => {
            let model_tables = visible_tables(model);
            let only_cache: Vec<&String> = cache_tables.difference(&model_tables).collect();
            let only_model: Vec<&String> = model_tables.difference(&cache_tables).collect();
            if only_cache.is_empty() && only_model.is_empty() {
                format!(
                    "local data cache {entry} matches the model definition ({} tables); data loading comes later",
                    cache_tables.len()
                )
            } else {
                diags.push(
                    Diagnostic::warning(
                        codes::CACHE_ABF,
                        format!(
                            "local data cache {entry} differs from the model definition (only in the cache: {only_cache:?}; only in the definition: {only_model:?}); it may be stale"
                        ),
                    )
                    .with_origin(origin),
                );
                return Some(part);
            }
        }
    };
    diags.push(Diagnostic::info(codes::CACHE_ABF, message).with_origin(origin));
    Some(part)
}

/// Most table names listed in a `cacheAbf` part.
const CACHE_PART_MAX_TABLES: usize = 1000;

fn cache_part(
    entry: &str,
    size: u64,
    cached: &Loaded,
    tables: &BTreeSet<String>,
    matches: Option<bool>,
) -> MetadataPart {
    let mut json = serde_json::Map::new();
    json.insert("size".into(), Value::from(size));
    if let ModelSource::Abf {
        compressed,
        archive,
        schema_version,
        ..
    } = &cached.source
    {
        json.insert("xpress9".into(), Value::from(*compressed));
        json.insert(
            "perFileCompression".into(),
            Value::from(archive.is_compressed()),
        );
        if let Some(v) = schema_version {
            json.insert("schemaVersion".into(), Value::from(*v));
        }
    }
    let format = cached
        .model
        .extra
        .get("vertipaq.metadataFormat")
        .and_then(PropValue::as_str)
        .unwrap_or("sqlite");
    json.insert("metadataFormat".into(), Value::from(format));
    if let Some(level) = cached.model.compatibility_level() {
        json.insert("compatibilityLevel".into(), Value::from(level));
    }
    let names: Vec<Value> = cached
        .model
        .tables
        .iter()
        .filter(|t| tables.contains(&t.name.to_lowercase()))
        .take(CACHE_PART_MAX_TABLES)
        .map(|t| Value::from(t.name.clone()))
        .collect();
    json.insert("tableCount".into(), Value::from(tables.len()));
    json.insert("tables".into(), Value::Array(names));
    json.insert(
        "matchesDefinition".into(),
        matches.map_or(Value::Null, Value::from),
    );
    MetadataPart {
        path: entry.to_owned(),
        kind: "cacheAbf".into(),
        encoding: None,
        text: None,
        json: Some(Value::Object(json)),
        redacted: Vec::new(),
    }
}

/// A copy of `model` without the `vertipaq.*` storage facts, which are not
/// TMDL/TMSL properties.
pub(crate) fn without_storage_facts(model: &Model) -> Model {
    let mut model = model.clone();
    let strip = |extra: &mut pbi_model::PropertyBag| {
        extra.retain(|k, _: &mut PropValue| !k.starts_with("vertipaq."));
    };
    strip(&mut model.extra);
    for t in &mut model.tables {
        strip(&mut t.extra);
        for c in &mut t.columns {
            strip(&mut c.extra);
        }
        for m in &mut t.measures {
            strip(&mut m.extra);
        }
        for p in &mut t.partitions {
            strip(&mut p.extra);
        }
        for h in &mut t.hierarchies {
            strip(&mut h.extra);
            for l in &mut h.levels {
                strip(&mut l.extra);
            }
        }
    }
    for r in &mut model.relationships {
        strip(&mut r.extra);
    }
    model
}

/// Removes TMSL layout shadows from a model read from TMSL before it is
/// written as TMDL.
///
/// `pbi-tmsl` keeps the original JSON of properties whose source spelling
/// differs from its canonical output (multi-line strings, enum spelling,
/// explicit defaults) in `extra` under the property's own key, plus empty
/// `List` markers for absent keys. Those are TMSL layout, not model data;
/// written as TMDL they would duplicate typed properties. A shadow is an
/// `extra` JSON value whose key the TMSL writer also emits from the typed
/// fields; empty arrays and absent markers carry no data either.
pub(crate) fn strip_tmsl_shadows(model: &mut Model) {
    let mut bare = model.clone();
    clear_extras(&mut bare);
    let json = pbi_tmsl::to_json(&bare, &pbi_tmsl::WriteOptions::default());
    let at = |v: Option<&Value>, key: &str, i: usize| {
        v.and_then(|v| v.get(key)).and_then(|a| a.get(i)).cloned()
    };
    let m = json.get("model").cloned();
    strip_bag(&mut model.extra, m.as_ref(), &[]);
    if let Some(db) = &mut model.database {
        strip_bag(&mut db.extra, Some(&json), &[]);
    }
    for (ti, t) in model.tables.iter_mut().enumerate() {
        let tj = at(m.as_ref(), "tables", ti);
        strip_bag(&mut t.extra, tj.as_ref(), &[]);
        for (i, c) in t.columns.iter_mut().enumerate() {
            strip_bag(&mut c.extra, at(tj.as_ref(), "columns", i).as_ref(), &[]);
        }
        for (i, x) in t.measures.iter_mut().enumerate() {
            strip_bag(&mut x.extra, at(tj.as_ref(), "measures", i).as_ref(), &[]);
        }
        for (i, h) in t.hierarchies.iter_mut().enumerate() {
            let hj = at(tj.as_ref(), "hierarchies", i);
            strip_bag(&mut h.extra, hj.as_ref(), &[]);
            for (j, l) in h.levels.iter_mut().enumerate() {
                strip_bag(&mut l.extra, at(hj.as_ref(), "levels", j).as_ref(), &[]);
            }
        }
        for (i, p) in t.partitions.iter_mut().enumerate() {
            let pj = at(tj.as_ref(), "partitions", i);
            let typed_source = pj.as_ref().and_then(|p| p.get("source"));
            if let Some(PropValue::Json(Value::Object(rest))) = p.extra.get_mut("source") {
                rest.retain(|k, _| typed_source.and_then(|s| s.get(k)).is_none());
                if rest.is_empty() {
                    p.extra.shift_remove("source");
                }
            }
            strip_bag(&mut p.extra, pj.as_ref(), &["source"]);
        }
    }
    for (i, r) in model.relationships.iter_mut().enumerate() {
        strip_bag(
            &mut r.extra,
            at(m.as_ref(), "relationships", i).as_ref(),
            &[],
        );
    }
    for (i, e) in model.expressions.iter_mut().enumerate() {
        strip_bag(&mut e.extra, at(m.as_ref(), "expressions", i).as_ref(), &[]);
    }
    for (i, r) in model.roles.iter_mut().enumerate() {
        let rj = at(m.as_ref(), "roles", i);
        strip_bag(&mut r.extra, rj.as_ref(), &[]);
        for (j, p) in r.table_permissions.iter_mut().enumerate() {
            strip_bag(
                &mut p.extra,
                at(rj.as_ref(), "tablePermissions", j).as_ref(),
                &[],
            );
        }
    }
    for (i, c) in model.cultures.iter_mut().enumerate() {
        strip_bag(&mut c.extra, at(m.as_ref(), "cultures", i).as_ref(), &[]);
    }
    for (i, p) in model.perspectives.iter_mut().enumerate() {
        strip_bag(
            &mut p.extra,
            at(m.as_ref(), "perspectives", i).as_ref(),
            &[],
        );
    }
}

fn strip_bag(extra: &mut pbi_model::PropertyBag, typed: Option<&Value>, keep: &[&str]) {
    extra.retain(|k, v| {
        if keep.contains(&k.as_str()) {
            return true;
        }
        match v {
            PropValue::List(items) => !items.is_empty(),
            PropValue::Json(Value::Array(items)) if items.is_empty() => false,
            PropValue::Json(_) => typed.and_then(|t| t.get(k)).is_none(),
            _ => true,
        }
    });
}

fn clear_extras(model: &mut Model) {
    model.extra.clear();
    if let Some(db) = &mut model.database {
        db.extra.clear();
    }
    for t in &mut model.tables {
        t.extra.clear();
        t.columns.iter_mut().for_each(|c| c.extra.clear());
        t.measures.iter_mut().for_each(|m| m.extra.clear());
        t.partitions.iter_mut().for_each(|p| p.extra.clear());
        for h in &mut t.hierarchies {
            h.extra.clear();
            h.levels.iter_mut().for_each(|l| l.extra.clear());
        }
    }
    model.relationships.iter_mut().for_each(|r| r.extra.clear());
    model.expressions.iter_mut().for_each(|e| e.extra.clear());
    for r in &mut model.roles {
        r.extra.clear();
        r.table_permissions.iter_mut().for_each(|p| p.extra.clear());
    }
    model.cultures.iter_mut().for_each(|c| c.extra.clear());
    model.perspectives.iter_mut().for_each(|p| p.extra.clear());
}
