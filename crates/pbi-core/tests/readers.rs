//! Readers (`open_path` / `open_bytes`) and writers (`save_as_pbip` /
//! `export_pbit`) over the committed Acme Outfitters example, the Microsoft
//! sample PBIX/PBIT files and synthetic inputs.
//!
//! Fixtures are found through `$PBIVIEW_REFERENCE`, falling back to the
//! repository's `reference/` folder. The fetched samples
//! (`samples/downloads/`) are skipped with a `SKIP:` message when missing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use pbi_core::container::{ContainerError, UnsafePathReason};
use pbi_core::reader::part_kind;
use pbi_core::redact::REDACTED;
use pbi_core::{
    CoreError, ModelSource, OpenOptions, OpenedDocument, ReportSource, SaveOptions, codes,
    export_pbit, open_bytes, open_path, save_as_pbip,
};
use pbi_model::{
    Condition, CrossFilter, DatasetReference, Document, FieldExpr, Literal, Model, ModelStatus,
    PropValue, Report, Severity, SourceFormat, VisualKind,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

/// A path inside the committed Acme Outfitters example.
fn example(rel: &str) -> PathBuf {
    reference_root().join("examples/acme-outfitters").join(rel)
}

/// The `samples/microsoft/expected.json` entries whose file is present, as
/// `(path, entry)`; missing (fetched) files print SKIP.
fn microsoft_samples() -> Vec<(PathBuf, Value)> {
    let path = reference_root().join("samples/microsoft/expected.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: {} missing", path.display());
        return Vec::new();
    };
    let expected: Value = serde_json::from_str(&text).unwrap();
    expected["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| {
            let file = reference_root().join(f["file"].as_str().unwrap());
            if file.is_file() {
                Some((file, f.clone()))
            } else {
                eprintln!(
                    "SKIP: {} missing (run scripts/fetch-samples.sh)",
                    file.display()
                );
                None
            }
        })
        .collect()
}

/// One Microsoft sample by its `expected.json` id, with its facts.
fn microsoft_sample(id: &str) -> Option<(PathBuf, Value)> {
    let path = reference_root().join("samples/microsoft/expected.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: {} missing", path.display());
        return None;
    };
    let expected: Value = serde_json::from_str(&text).unwrap();
    let facts = expected["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no sample {id} in expected.json"))
        .clone();
    let file = reference_root().join(facts["file"].as_str().unwrap());
    if file.is_file() {
        Some((file, facts))
    } else {
        eprintln!("SKIP: {} missing", file.display());
        None
    }
}

/// The entries of a ZIP package, by name.
fn zip_entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
        out.insert(entry.name().unwrap().to_string(), bytes);
    }
    out
}

fn open(path: &Path) -> OpenedDocument {
    open_path(path, &OpenOptions::default())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbi-core-readers-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let opts = SimpleFileOptions::default();
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(*name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn errors(doc: &Document) -> Vec<String> {
    doc.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{} {}", d.code, d.message))
        .collect()
}

fn has_diag(doc: &Document, code: &str) -> bool {
    doc.diagnostics.iter().any(|d| d.code == code)
}

/// Every object ID is assigned and unique.
fn assert_ids_unique(doc: &Document) {
    let mut seen = HashSet::new();
    let mut count = 0;
    doc.walk(&mut |id, path| {
        assert!(id.is_assigned(), "unassigned id at {path:?}");
        assert!(seen.insert(id), "duplicate id {id:?} at {path:?}");
        count += 1;
        ControlFlow::Continue(())
    });
    assert!(count > 0);
}

/// JSON of a value without `id` and `origin` keys (IDs and source
/// locations differ between formats).
fn normalized<T: serde::Serialize>(value: &T) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                map.remove("id");
                map.remove("origin");
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(value).unwrap();
    strip(&mut v);
    v
}

/// [`normalized`] with every `extra` value in one neutral JSON form. TMDL
/// keeps untyped properties as text, expressions or raw objects and TMSL as
/// JSON (a measure's `formatStringDefinition` as `{"expression": …}`), so
/// the same property has a different shape after a conversion.
fn normalized_across_formats<T: serde::Serialize>(value: &T) -> Value {
    fn neutral(v: &PropValue) -> Value {
        match v {
            PropValue::Json(Value::Object(map)) if map.len() == 1 => match map.get("expression") {
                Some(Value::String(s)) => Value::String(s.clone()),
                Some(Value::Array(lines)) => Value::String(
                    lines
                        .iter()
                        .map(|l| l.as_str().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => v.to_json(),
            },
            other => other.to_json(),
        }
    }
    fn walk(v: &mut Value) {
        match v {
            Value::Object(map) => {
                if let Some(Value::Object(extra)) = map.get_mut("extra") {
                    for value in extra.values_mut() {
                        let prop: PropValue = serde_json::from_value(value.clone()).unwrap();
                        *value = neutral(&prop);
                    }
                }
                for (key, child) in map.iter_mut() {
                    if key != "extra" {
                        walk(child);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            _ => {}
        }
    }
    let mut v = normalized(value);
    walk(&mut v);
    v
}

fn model(doc: &OpenedDocument) -> &Model {
    doc.document.model.as_ref().expect("model")
}

fn files_under(root: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir_files(root, root)
}

fn walkdir_files(root: &Path, dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            out.extend(walkdir_files(root, &path));
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&path).unwrap());
        }
    }
    out
}

const MODEL_FOLDER: &str = "AcmeOutfitters.SemanticModel";
const OVERVIEW: &str = "Acme Outfitters - Overview.Report";
const STORES: &str = "Acme Outfitters - Stores.Report";
const ONLINE: &str = "Acme Outfitters - Online.Report";
/// `(page name, visual count)` per report, in page order.
const OVERVIEW_PAGES: [(&str, usize); 3] = [
    ("ef796b39c816a4afa4f1", 14),
    ("59126dbf5023ae95f4dc", 13),
    ("de11804d145ce7e78111", 12),
];
const STORES_PAGES: [(&str, usize); 3] = [
    ("ef3ce6b6fadf22960137", 14),
    ("50c645140e8e10eea677", 5),
    ("58db5b3f75d059fec1fb", 1),
];
const ONLINE_PAGES: [(&str, usize); 2] =
    [("a60af4d6244c3a6206a5", 14), ("95c48410a9ac03ae2a38", 7)];

fn page_visuals(report: &Report) -> Vec<(String, usize)> {
    report
        .pages
        .iter()
        .map(|p| (p.name.clone(), p.visuals.len()))
        .collect()
}

fn pages(expected: &[(&str, usize)]) -> Vec<(String, usize)> {
    expected
        .iter()
        .map(|(p, n)| ((*p).to_owned(), *n))
        .collect()
}

/// A copy of the Stores report bound to a remote semantic model
/// (`byConnection`), in `dir`.
fn remote_report_folder(dir: &Path) -> PathBuf {
    let folder = dir.join("Acme Outfitters - Live.Report");
    copy_dir(&example(STORES), &folder);
    std::fs::write(
        folder.join("definition.pbir"),
        r#"{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json",
  "version": "4.0",
  "datasetReference": {
    "byConnection": {
      "connectionString": "Data Source=powerbi://api.powerbi.com/v1.0/myorg/Acme%20Outfitters;initial catalog=AcmeOutfitters;integrated security=ClaimsToken;semanticmodelid=00000000-0000-4000-8000-0000000000ac"
    }
  }
}
"#,
    )
    .unwrap();
    folder
}

fn assert_acme_model(model: &Model) {
    let tables: Vec<&str> = model.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        tables,
        [
            "Orders",
            "Customers",
            "Products",
            "Stores",
            "Targets",
            "Returns",
            "Web Sessions",
            "Date",
            "Time Calc",
            "Products Never Sold",
            "Last Refreshed"
        ]
    );
    let measures: usize = model.tables.iter().map(|t| t.measures.len()).sum();
    assert_eq!(measures, 33);
    assert_eq!(model.relationships.len(), 8);
    let rel = &model.relationships[0];
    assert_eq!(rel.cross_filtering_behavior, CrossFilter::BothDirections);
    assert_eq!(
        (rel.from.table.as_str(), rel.from.column.as_str()),
        ("Orders", "Customer ID")
    );
    assert_eq!(
        (rel.to.table.as_str(), rel.to.column.as_str()),
        ("Customers", "Customer ID")
    );
    assert_eq!(model.roles.len(), 2);
    let param = model.expression("StorageContainerUrl").expect("parameter");
    assert!(param.is_parameter());
}

// ---------------------------------------------------------------------------
// PBIP: the Acme Outfitters example
// ---------------------------------------------------------------------------

#[test]
fn pbip_project_reads_model_and_three_reports() {
    let opened = open(&example(""));
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbip);
    assert_eq!(doc.model_status, ModelStatus::Local);
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
    assert!(
        doc.source
            .entries
            .contains(&"AcmeOutfitters.pbip".to_owned())
    );

    let m = model(&opened);
    assert_acme_model(m);
    assert_eq!(
        m.tables[0].origin.as_ref().unwrap().path,
        format!("{MODEL_FOLDER}/definition/tables/Orders.tmdl")
    );
    match &opened.model_source {
        Some(ModelSource::Tmdl {
            folder,
            definition_dir,
            files,
            ..
        }) => {
            assert_eq!(folder, MODEL_FOLDER);
            assert_eq!(definition_dir, &format!("{MODEL_FOLDER}/definition"));
            assert_eq!(files.len(), 17);
        }
        other => panic!("unexpected model source {other:?}"),
    }

    // Reports: the .pbip artifact first, then the others by folder name.
    let names: Vec<&str> = doc.reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Acme Outfitters - Overview",
            "Acme Outfitters - Online",
            "Acme Outfitters - Stores"
        ]
    );
    let folders = [OVERVIEW, ONLINE, STORES];
    let expected_pages: [&[(&str, usize)]; 3] = [&OVERVIEW_PAGES, &ONLINE_PAGES, &STORES_PAGES];
    for ((report, folder), expected) in doc.reports.iter().zip(folders).zip(expected_pages) {
        assert_eq!(page_visuals(report), pages(expected), "{folder}");
        assert_eq!(
            report.dataset_reference,
            DatasetReference::ByPath {
                path: format!("../{MODEL_FOLDER}")
            }
        );
        assert_eq!(report.active_page.as_deref(), Some(expected[0].0));
        assert_eq!(
            report.origin.as_ref().unwrap().path,
            format!("{folder}/definition/report.json")
        );
        assert_eq!(
            report.pages[0].visuals[0]
                .origin
                .as_ref()
                .unwrap()
                .path
                .split('/')
                .next(),
            Some(folder)
        );
    }

    // Overview: report-level `Orders[Status] NOT IN {'Cancelled'}`.
    let status = doc.reports[0]
        .filters
        .iter()
        .find(|f| f.field == Some(FieldExpr::column("Orders", "Status")))
        .expect("Status filter");
    match status.condition.as_ref().expect("condition") {
        Condition::Not(inner) => match inner.as_ref() {
            Condition::In { exprs, values } => {
                assert_eq!(exprs, &[FieldExpr::column("Orders", "Status")]);
                assert_eq!(values, &[vec![Literal::text("Cancelled")]]);
            }
            other => panic!("unexpected condition {other:?}"),
        },
        other => panic!("unexpected condition {other:?}"),
    }
    // Stores: `Stores[Channel] IN {'Retail', 'Outlet'}`.
    match doc.reports[2].filters[0]
        .condition
        .as_ref()
        .expect("condition")
    {
        Condition::In { exprs, values } => {
            assert_eq!(exprs, &[FieldExpr::column("Stores", "Channel")]);
            assert_eq!(
                values,
                &[vec![Literal::text("Retail")], vec![Literal::text("Outlet")]]
            );
        }
        other => panic!("unexpected condition {other:?}"),
    }

    assert_eq!(opened.report_sources.len(), 3);
    for (source, folder) in opened.report_sources.iter().zip(folders) {
        assert!(matches!(source, ReportSource::Pbir { .. }));
        assert_eq!(source.folder(), folder);
    }
    assert_ids_unique(doc);

    // Metadata parts.
    let pbism = opened.part("pbism").expect("definition.pbism");
    assert_eq!(pbism.path, format!("{MODEL_FOLDER}/definition.pbism"));
    assert_eq!(pbism.json.as_ref().unwrap()["version"], "4.2");
    let pbip = opened.part("pbip").expect(".pbip");
    assert_eq!(
        pbip.json.as_ref().unwrap()["artifacts"][0]["report"]["path"],
        OVERVIEW
    );
    let platforms: Vec<&str> = opened
        .parts
        .iter()
        .filter(|p| p.kind == "platform")
        .map(|p| {
            p.json.as_ref().unwrap()["metadata"]["type"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(platforms, ["SemanticModel", "Report", "Report", "Report"]);
    let theme = doc
        .extra_parts
        .iter()
        .find(|p| {
            p.path == format!("{OVERVIEW}/StaticResources/RegisteredResources/AcmeOutfitters.json")
        })
        .expect("theme listed");
    assert_eq!(theme.kind, "theme");
    assert!(
        !doc.extra_parts.iter().any(|p| p.path.ends_with(".tmdl")),
        "definitions are not extra parts"
    );

    let inspect = opened.inspect_json();
    assert_eq!(inspect["detection"]["format"]["kind"], "pbip");
    assert_eq!(inspect["modelSource"]["kind"], "tmdl");
    assert_eq!(inspect["document"]["modelStatus"]["status"], "local");
}

#[test]
fn pbip_file_opens_its_project() {
    let from_folder = open(&example(""));
    let from_file = open(&example("AcmeOutfitters.pbip"));
    assert_eq!(from_file.document.source.format, SourceFormat::Pbip);
    assert_eq!(
        normalized(&from_file.document.model),
        normalized(&from_folder.document.model)
    );
    assert_eq!(
        normalized(&from_file.document.reports),
        normalized(&from_folder.document.reports)
    );
}

#[test]
fn report_folders_open_alone_with_their_sibling_model() {
    for (folder, expected) in [
        (OVERVIEW, &OVERVIEW_PAGES[..]),
        (STORES, &STORES_PAGES[..]),
        (ONLINE, &ONLINE_PAGES[..]),
    ] {
        let opened = open(&example(folder));
        let doc = &opened.document;
        assert_eq!(doc.source.format, SourceFormat::ReportFolder, "{folder}");
        assert_eq!(doc.reports.len(), 1);
        assert_eq!(page_visuals(&doc.reports[0]), pages(expected), "{folder}");
        assert_eq!(
            doc.reports[0].origin.as_ref().unwrap().path,
            "definition/report.json"
        );
        // byPath `../AcmeOutfitters.SemanticModel` resolves next to the folder.
        assert_eq!(doc.model_status, ModelStatus::Local, "{folder}");
        assert_acme_model(model(&opened));
        assert!(has_diag(doc, codes::EXTERNAL_MODEL));
        assert!(errors(doc).is_empty(), "{:?}", errors(doc));
        assert_ids_unique(doc);
    }
}

#[test]
fn semantic_model_folder_opens_alone() {
    let opened = open(&example(MODEL_FOLDER));
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::SemanticModelFolder);
    assert_eq!(doc.model_status, ModelStatus::Local);
    assert!(doc.reports.is_empty());
    assert_acme_model(model(&opened));
    assert_eq!(
        model(&opened).tables[0].origin.as_ref().unwrap().path,
        "definition/tables/Orders.tmdl"
    );
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
    assert_ids_unique(doc);
}

#[test]
fn loose_tmdl_folder_and_model_tmdl_file() {
    let definition = example(&format!("{MODEL_FOLDER}/definition"));
    let opened = open(&definition);
    assert_eq!(opened.document.source.format, SourceFormat::TmdlFolder);
    assert_acme_model(model(&opened));
    // `model.tmdl` opens its folder.
    let via_file = open(&definition.join("model.tmdl"));
    assert_eq!(via_file.document.source.format, SourceFormat::TmdlFolder);
    assert_eq!(
        normalized(&via_file.document.model),
        normalized(&opened.document.model)
    );
}

#[test]
fn remote_report_folder_is_remote() {
    let dir = tmp_dir();
    let opened = open(&remote_report_folder(dir.path()));
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::ReportFolder);
    assert!(doc.model.is_none());
    assert_eq!(
        doc.model_status,
        ModelStatus::Remote {
            description: "remote, not available offline".into()
        }
    );
    assert_eq!(doc.reports.len(), 1);
    let report = &doc.reports[0];
    // The name comes from the folder's `.platform` display name.
    assert_eq!(report.name, "Acme Outfitters - Stores");
    assert_eq!(page_visuals(report), pages(&STORES_PAGES));
    match &report.dataset_reference {
        DatasetReference::ByConnection {
            connection_string, ..
        } => {
            let cs = connection_string.as_deref().unwrap();
            assert!(
                cs.starts_with("Data Source=powerbi://api.powerbi.com/"),
                "{cs}"
            );
        }
        other => panic!("unexpected dataset reference {other:?}"),
    }
    assert!(has_diag(doc, codes::REMOTE_MODEL));
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
    assert_ids_unique(doc);
}

// ---------------------------------------------------------------------------
// PBIX and PBIT: the Microsoft samples
// ---------------------------------------------------------------------------

/// The report and model of an opened sample equal the facts recorded in
/// `expected.json` (cross-checked there against an independent decoder and
/// a raw parse of the package).
fn assert_sample_facts(id: &Value, opened: &OpenedDocument, facts: &Value) {
    let doc = &opened.document;
    let m = model(opened);
    let expected = &facts["model"];
    assert_eq!(
        m.compatibility_level(),
        expected["compatibilityLevel"].as_i64(),
        "{id}"
    );
    let tables: Vec<Value> = m
        .tables
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "kind": t.kind(),
                "hidden": t.is_hidden,
                "columns": t.columns.len(),
                "measures": t.measures.len(),
            })
        })
        .collect();
    let expected_tables: Vec<Value> = expected["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            json!({
                "name": t["name"],
                "kind": t["kind"],
                "hidden": t["hidden"],
                "columns": t["columns"],
                "measures": t["measures"],
            })
        })
        .collect();
    assert_eq!(tables, expected_tables, "{id}");
    let counts = &expected["counts"];
    assert_eq!(
        json!(m.relationships.len()),
        counts["relationships"],
        "{id}"
    );
    assert_eq!(json!(m.expressions.len()), counts["expressions"], "{id}");
    assert_eq!(json!(m.parameters().count()), counts["parameters"], "{id}");
    let mut measures: Vec<String> = m
        .tables
        .iter()
        .flat_map(|t| {
            t.measures
                .iter()
                .map(move |ms| format!("{}[{}]", t.name, ms.name))
        })
        .collect();
    measures.sort();
    if let Some(names) = expected["measures"]["names"].as_array() {
        let mut names: Vec<String> = names
            .iter()
            .map(|n| n.as_str().unwrap().to_owned())
            .collect();
        names.sort();
        assert_eq!(measures, names, "{id}");
    }
    assert_eq!(json!(measures.len()), expected["measures"]["count"], "{id}");

    assert_eq!(doc.reports.len(), 1, "{id}");
    let report = &doc.reports[0];
    let facts = &facts["report"];
    assert_eq!(json!(report.name), facts["name"], "{id}");
    assert_eq!(json!(report.active_page), facts["activePage"], "{id}");
    let pages: Vec<Value> = report
        .pages
        .iter()
        .map(|p| {
            let mut by_type = BTreeMap::new();
            for v in &p.visuals {
                let kind = match v.kind {
                    VisualKind::Group => "group".to_owned(),
                    VisualKind::Visual => v.visual_type.clone(),
                };
                *by_type.entry(kind).or_insert(0) += 1;
            }
            json!({
                "name": p.name,
                "displayName": p.display_name,
                "width": p.width,
                "height": p.height,
                "hidden": p.is_hidden,
                "visuals": p.visuals.len(),
                "visualsByType": by_type,
            })
        })
        .collect();
    assert_eq!(json!(pages), facts["pages"], "{id}");
    assert_eq!(report.dataset_reference, DatasetReference::Embedded, "{id}");
}

#[test]
fn microsoft_samples_match_expected_json() {
    let samples = microsoft_samples();
    for (path, facts) in &samples {
        let id = &facts["id"];
        let opened = open(path);
        let doc = &opened.document;
        let format = match facts["detect"]["format"].as_str().unwrap() {
            "pbix" => SourceFormat::Pbix,
            "pbit" => SourceFormat::Pbit,
            other => panic!("{id}: unexpected format {other}"),
        };
        assert_eq!(doc.source.format, format, "{id}");
        assert_eq!(doc.model_status, ModelStatus::Local, "{id}");
        assert!(errors(doc).is_empty(), "{id}: {:?}", errors(doc));
        match facts["model"]["source"].as_str().unwrap() {
            "abf" => assert!(
                matches!(&opened.model_source, Some(ModelSource::Abf { entry, compressed: true, .. }) if entry == "DataModel"),
                "{id}: {:?}",
                opened.model_source
            ),
            "tmsl" => assert!(
                matches!(&opened.model_source, Some(ModelSource::Tmsl { entry, .. }) if entry == "DataModelSchema"),
                "{id}: {:?}",
                opened.model_source
            ),
            other => panic!("{id}: unexpected model source {other}"),
        }
        match facts["detect"]["reportFlavour"].as_str().unwrap() {
            "pbir" => assert!(
                matches!(&opened.report_sources[..], [ReportSource::Pbir { folder, .. }] if folder == "Report"),
                "{id}"
            ),
            "legacyLayout" => assert!(
                matches!(&opened.report_sources[..], [ReportSource::Legacy { entry, .. }] if entry == "Report/Layout"),
                "{id}"
            ),
            other => panic!("{id}: unexpected report flavour {other}"),
        }
        assert_sample_facts(id, &opened, facts);
        assert!(
            model(&opened)
                .tables
                .iter()
                .all(|t| !t.name.starts_with("H$") && !t.name.starts_with("R$")),
            "{id}"
        );
        assert!(doc.source.entries.contains(&"SecurityBindings".to_owned()));
        assert!(has_diag(doc, codes::OPAQUE), "{id}");
        assert_ids_unique(doc);
    }
    if reference_root().join("samples/microsoft").is_dir() {
        assert!(samples.len() >= 4);
    }
}

#[test]
fn unzipped_pbix_folder() {
    let Some((pbix, facts)) = microsoft_sample("revenue-opportunities") else {
        return;
    };
    let dir = tmp_dir();
    let root = dir.path().join("Revenue Opportunities");
    for (name, bytes) in zip_entries(&pbix) {
        let path = root.join(&name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let opened = open(&root);
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbix);
    assert_eq!(opened.package_version(), Some("1.32"));
    let settings = opened.part("settings").expect("Settings");
    assert!(settings.json.is_some());
    assert!(opened.part("metadata").is_some());
    assert!(opened.part("diagramLayout").is_some());
    assert!(opened.part("connections").is_some());
    assert!(
        matches!(&opened.report_sources[..], [ReportSource::Pbir { folder, .. }] if folder == "Report")
    );
    assert_eq!(doc.model_status, ModelStatus::Local);
    assert!(matches!(
        opened.model_source,
        Some(ModelSource::Abf {
            compressed: true,
            ..
        })
    ));
    assert_sample_facts(&facts["id"], &opened, &facts);
    assert!(has_diag(doc, codes::OPAQUE));
    assert!(
        doc.extra_parts
            .iter()
            .any(|p| p.path == "SecurityBindings" && p.kind == "securityBindings")
    );
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
    assert_ids_unique(doc);

    // The unzipped folder and the package give the same document.
    let zipped = open(&pbix);
    assert_eq!(normalized(&doc.model), normalized(&zipped.document.model));
    assert_eq!(
        normalized(&doc.reports[0].pages),
        normalized(&zipped.document.reports[0].pages)
    );
}

#[test]
fn pbix_without_data_model_still_opens_its_report() {
    let Some((pbix, facts)) = microsoft_sample("revenue-opportunities") else {
        return;
    };
    let parts: Vec<(String, Vec<u8>)> = zip_entries(&pbix)
        .into_iter()
        .filter(|(p, _)| p != "DataModel" && p != "SecurityBindings")
        .collect();
    let refs: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let opened = open_bytes("Clean.pbix", zip_of(&refs), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbix);
    assert_eq!(doc.reports.len(), 1);
    assert_eq!(doc.reports[0].name, "Clean");
    assert_eq!(
        doc.reports[0].pages.len(),
        facts["report"]["pages"].as_array().unwrap().len()
    );
    assert_eq!(
        doc.reports[0].origin.as_ref().unwrap().path,
        "Report/definition/report.json"
    );
    // `Connections` only lists published artifacts: not a live connection.
    assert_eq!(doc.model_status, ModelStatus::Missing);
    assert!(has_diag(doc, codes::NO_MODEL));
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
}

#[test]
fn pbix_schema_options_and_save_as_pbip() {
    let Some((path, _)) = microsoft_sample("revenue-opportunities") else {
        return;
    };
    let opened = open(&path);
    let doc = &opened.document;
    let m = model(&opened);
    assert_eq!(doc.reports[0].name, "Revenue Opportunities");
    assert!(doc.reports[0].theme.custom_theme.is_some());

    // Schema loading can be switched off; the report still opens.
    let opts = OpenOptions {
        load_pbix_schema: false,
        ..OpenOptions::default()
    };
    let light = open_path(&path, &opts).unwrap();
    assert!(light.document.model.is_none());
    assert!(has_diag(&light.document, codes::NO_MODEL));
    assert_eq!(
        normalized(&light.document.reports[0].pages),
        normalized(&doc.reports[0].pages)
    );

    // Internal VertiPaq tables on request.
    let opts = OpenOptions {
        include_system_tables: true,
        ..OpenOptions::default()
    };
    let full = open_path(&path, &opts).unwrap();
    assert!(model(&full).tables.len() > m.tables.len());

    // PBIX → PBIP: the schema is written as canonical TMDL (no storage
    // facts) and the report binds to it by path.
    let dir = tmp_dir();
    let out = dir.path().join("from-pbix");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    let tmdl = std::fs::read_to_string(
        out.join("Revenue Opportunities.SemanticModel/definition/model.tmdl"),
    )
    .unwrap();
    assert!(!tmdl.contains("vertipaq."), "{tmdl}");
    let project = open(&out);
    assert!(
        errors(&project.document).is_empty(),
        "{:?}",
        errors(&project.document)
    );
    assert_eq!(project.document.model_status, ModelStatus::Local);
    let names = |m: &Model| m.tables.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(model(&project)), names(m));
    assert_eq!(
        normalized(&project.document.reports[0].pages),
        normalized(&doc.reports[0].pages)
    );
}

/// The COVID-19 template's `DataModelSchema` is UTF-16LE TMSL (CRLF, no
/// BOM): it reads with the expected facts and writes back in the same
/// encoding and layout, JSON-identical (TMSL keys are written in canonical
/// TOM order, so bytes may differ in key order).
#[test]
fn pbit_data_model_schema_is_utf16_tmsl_and_round_trips() {
    let Some((path, facts)) = microsoft_sample("covid-19-us-tracking") else {
        return;
    };
    let bytes = zip_entries(&path).remove("DataModelSchema").unwrap();
    assert_eq!(&bytes[..2], b"{\0", "UTF-16LE without BOM");
    let loaded = pbi_tmsl::read_data_model_schema(&bytes).unwrap();
    assert_eq!(loaded.format.encoding, pbi_tmsl::TextEncoding::Utf16Le);
    assert!(!loaded.format.bom);
    assert_eq!(
        loaded.compatibility_level,
        facts["model"]["compatibilityLevel"].as_i64()
    );
    let names: Vec<&str> = loaded
        .model
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    let expected: Vec<&str> = facts["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, expected);
    assert_eq!(
        loaded.model.relationships.len() as u64,
        facts["model"]["counts"]["relationships"].as_u64().unwrap()
    );
    let written = pbi_tmsl::write_bytes(&loaded.model, &loaded.write_options());
    assert_eq!(&written[..4], b"{\0\r\0", "UTF-16LE, CRLF");
    assert_eq!(written.len(), bytes.len());
    let json = |b: &[u8]| -> Value {
        serde_json::from_str(&pbi_tmsl::decode_text(b).unwrap().text).unwrap()
    };
    assert_eq!(json(&written), json(&bytes));
    let again = pbi_tmsl::read_data_model_schema(&written).unwrap();
    assert_eq!(again.model, loaded.model);
    assert_eq!(again.format, loaded.format);
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

#[test]
fn pbip_save_round_trip_is_byte_identical() {
    let source = example("");
    let opened = open(&source);
    let dir = tmp_dir();
    let out = dir.path().join("project");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();

    // TMDL and PBIR files (and the project metadata) are byte-identical.
    let original = files_under(&source);
    let written = files_under(&out);
    for (path, bytes) in &original {
        let interesting = path.ends_with(".tmdl")
            || path.contains(".Report/")
            || path.ends_with(".pbip")
            || path.ends_with("definition.pbism")
            || path.ends_with(".platform");
        if interesting {
            let got = written
                .get(path)
                .unwrap_or_else(|| panic!("{path} not written"));
            assert!(got == bytes, "{path} differs");
        }
    }
    let extra: Vec<&String> = written
        .keys()
        .filter(|p| !original.contains_key(*p) && p.as_str() != ".gitignore")
        .collect();
    assert!(extra.is_empty(), "unexpected files {extra:?}");

    // Re-open: same model and report ASTs.
    let again = open(&out);
    assert_eq!(again.document.source.format, SourceFormat::Pbip);
    assert_eq!(
        normalized(&again.document.model),
        normalized(&opened.document.model)
    );
    assert_eq!(
        normalized(&again.document.reports),
        normalized(&opened.document.reports)
    );

    // Refuses to write into a non-empty folder unless asked to.
    let err = save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap_err();
    assert!(matches!(err, CoreError::Save(_)), "{err}");
    save_as_pbip(&opened, &out, &SaveOptions { overwrite: true }).unwrap();
}

#[test]
fn edited_model_rewrites_only_the_edited_file() {
    let source = example("");
    let mut opened = open(&source);
    let table = opened
        .document
        .model
        .as_mut()
        .unwrap()
        .table_mut("Last Refreshed")
        .unwrap();
    table.description = Some("Edited in a test".into());
    let dir = tmp_dir();
    let out = dir.path().join("edited");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    let original = files_under(&source);
    let written = files_under(&out);
    let changed: Vec<&String> = original
        .iter()
        .filter(|(p, b)| p.ends_with(".tmdl") && written.get(*p) != Some(*b))
        .map(|(p, _)| p)
        .collect();
    assert_eq!(
        changed,
        [&format!(
            "{MODEL_FOLDER}/definition/tables/Last Refreshed.tmdl"
        )]
    );
    let again = open(&out);
    assert_eq!(
        again
            .document
            .model
            .as_ref()
            .unwrap()
            .table("Last Refreshed")
            .unwrap()
            .description
            .as_deref(),
        Some("Edited in a test")
    );
}

#[test]
fn pbit_export_round_trip() {
    let opened = open(&example(""));
    let dir = tmp_dir();
    let pbit = dir.path().join("AcmeOutfitters.pbit");
    export_pbit(&opened, &pbit).unwrap();

    let bytes = std::fs::read(&pbit).unwrap();
    let zip = pbi_core::container::ZipContainer::from_bytes(bytes, Default::default()).unwrap();
    use pbi_core::container::Container;
    for entry in [
        "Version",
        "[Content_Types].xml",
        "DataModelSchema",
        "Settings",
        "Metadata",
        "Report/definition/report.json",
        "Report/definition/pages/pages.json",
        "Report/StaticResources/RegisteredResources/AcmeOutfitters.json",
    ] {
        assert!(zip.is_file(entry), "missing {entry}");
    }
    assert!(!zip.is_file("Report/definition.pbir"));
    assert!(!zip.is_file("DataModel"));
    let schema = zip.read("DataModelSchema").unwrap();
    assert_eq!(&schema[..2], b"{\0", "UTF-16LE without BOM");
    assert_eq!(zip.read_text("Version").unwrap().text, "1.32");

    let again = open(&pbit);
    let doc = &again.document;
    assert_eq!(doc.source.format, SourceFormat::Pbit);
    assert_eq!(doc.model_status, ModelStatus::Local);
    assert!(matches!(again.model_source, Some(ModelSource::Tmsl { .. })));
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
    assert_eq!(
        normalized_across_formats(&doc.model),
        normalized_across_formats(&opened.document.model)
    );
    assert_eq!(doc.reports.len(), 1);
    let (a, b) = (&doc.reports[0], &opened.document.reports[0]);
    assert_eq!(a.dataset_reference, DatasetReference::Embedded);
    assert_eq!(a.name, "AcmeOutfitters");
    assert_eq!(normalized(&a.pages), normalized(&b.pages));
    assert_eq!(normalized(&a.filters), normalized(&b.filters));
    assert_eq!(normalized(&a.theme), normalized(&b.theme));
    assert_eq!(a.settings, b.settings);
    assert_ids_unique(doc);

    // The PBIT saves back to a PBIP whose report binds to the model.
    let out = dir.path().join("from-pbit");
    save_as_pbip(&again, &out, &SaveOptions::default()).unwrap();
    let project = open(&out);
    assert_eq!(project.document.model_status, ModelStatus::Local);
    assert_eq!(
        normalized(&project.document.model),
        normalized(&opened.document.model)
    );
    assert_eq!(
        project.document.reports[0].dataset_reference,
        DatasetReference::ByPath {
            path: "../AcmeOutfitters.SemanticModel".into()
        }
    );
    assert_eq!(
        normalized(&project.document.reports[0].pages),
        normalized(&b.pages)
    );
}

// ---------------------------------------------------------------------------
// Synthetic inputs
// ---------------------------------------------------------------------------

/// A one-page legacy layout with a card bound to `Orders[Total]`.
fn legacy_layout() -> String {
    let visual_config = json!({
        "name": "card1",
        "layouts": [{"id": 0, "position": {"x": 10, "y": 20, "z": 0, "width": 300, "height": 120}}],
        "singleVisual": {
            "visualType": "card",
            "projections": {"Values": [{"queryRef": "Orders.Total"}]},
            "prototypeQuery": {
                "Version": 2,
                "From": [{"Name": "o", "Entity": "Orders", "Type": 0}],
                "Select": [{
                    "Measure": {"Expression": {"SourceRef": {"Source": "o"}}, "Property": "Total"},
                    "Name": "Orders.Total"
                }]
            }
        }
    });
    json!({
        "id": 0,
        "sections": [{
            "id": 0,
            "name": "ReportSection1",
            "displayName": "Overview",
            "displayOption": 1,
            "width": 1280,
            "height": 720,
            "filters": "[]",
            "config": "{}",
            "visualContainers": [{
                "x": 10, "y": 20, "z": 0, "width": 300, "height": 120,
                "config": visual_config.to_string(),
                "filters": "[]"
            }]
        }],
        "config": "{\"version\":\"5.43\"}",
        "layoutOptimization": 0
    })
    .to_string()
}

fn sample_bim() -> String {
    json!({
        "name": "SemanticModel",
        "compatibilityLevel": 1567,
        "model": {
            "culture": "en-US",
            "tables": [{
                "name": "Orders",
                "columns": [{"name": "Order ID", "dataType": "string", "sourceColumn": "Order ID"}],
                "measures": [{"name": "Total", "expression": "COUNTROWS(Orders)"}],
                "partitions": [{
                    "name": "Orders",
                    "mode": "import",
                    "source": {"type": "m", "expression": "let\n    Source = #table({\"Order ID\"}, {{\"SO-1\"}})\nin\n    Source"}
                }]
            }]
        }
    })
    .to_string()
}

#[test]
fn legacy_layout_pbix_opens_even_when_the_data_model_is_unreadable() {
    let layout = utf16le(&legacy_layout());
    let pbix = zip_of(&[
        ("Version", &utf16le("1.28")),
        ("[Content_Types].xml", b"<Types/>"),
        ("Report/Layout", &layout),
        ("DataModel", b"definitely not an ABF backup"),
    ]);
    let opened = open_bytes("Legacy.pbix", pbix.clone(), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbix);
    assert!(doc.model.is_none());
    assert_eq!(doc.model_status, ModelStatus::Missing);
    assert!(has_diag(doc, codes::MODEL_FAILED));
    assert_eq!(doc.reports.len(), 1);
    let report = &doc.reports[0];
    assert_eq!(report.name, "Legacy");
    assert_eq!(report.dataset_reference, DatasetReference::Embedded);
    assert_eq!(report.pages.len(), 1);
    assert_eq!(report.pages[0].display_name, "Overview");
    assert_eq!(report.pages[0].visuals.len(), 1);
    assert_eq!(
        report.pages[0].visuals[0].origin.as_ref().unwrap().path,
        "Report/Layout"
    );
    assert!(matches!(
        &opened.report_sources[..],
        [ReportSource::Legacy { entry, .. }] if entry == "Report/Layout"
    ));
    assert_eq!(opened.package_version(), Some("1.28"));
    assert_ids_unique(doc);

    let quiet = OpenOptions {
        load_pbix_schema: false,
        ..OpenOptions::default()
    };
    let opened = open_bytes("Legacy.pbix", pbix, &quiet).unwrap();
    assert!(errors(&opened.document).is_empty());
    assert!(has_diag(&opened.document, codes::NO_MODEL));
}

#[test]
fn legacy_pbit_round_trips_through_pbit_and_pbir_legacy() {
    let pbit = zip_of(&[
        ("Version", &utf16le("1.28")),
        ("DataModelSchema", &utf16le(&sample_bim())),
        ("Report/Layout", &utf16le(&legacy_layout())),
    ]);
    let opened = open_bytes("Template.pbit", pbit, &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbit);
    assert_eq!(doc.model_status, ModelStatus::Local);
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));
    assert_eq!(model(&opened).tables[0].measures[0].name, "Total");

    let dir = tmp_dir();
    let again_path = dir.path().join("again.pbit");
    export_pbit(&opened, &again_path).unwrap();
    let again = open(&again_path);
    assert_eq!(normalized(&again.document.model), normalized(&doc.model));
    assert_eq!(
        normalized(&again.document.reports[0].pages),
        normalized(&doc.reports[0].pages)
    );

    // Legacy layouts are saved as a PBIR-Legacy `report.json`.
    let out = dir.path().join("project");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    assert!(out.join("Template.Report/report.json").is_file());
    let project = open(&out);
    assert_eq!(project.document.model_status, ModelStatus::Local);
    assert!(matches!(
        &project.report_sources[..],
        [ReportSource::Legacy { entry, .. }] if entry == "Template.Report/report.json"
    ));
    assert_eq!(
        normalized(&project.document.reports[0].pages),
        normalized(&doc.reports[0].pages)
    );
    assert_eq!(
        project.document.reports[0].dataset_reference,
        DatasetReference::ByPath {
            path: "../Template.SemanticModel".into()
        }
    );
    // The PBIR-Legacy report folder alone.
    let folder = open(&out.join("Template.Report"));
    assert_eq!(folder.document.source.format, SourceFormat::PbirLegacy);
    assert_eq!(folder.document.model_status, ModelStatus::Local);
}

/// A minimal PBIR report under `Report/`.
fn pbir_report_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "Report/definition/version.json",
            br#"{"version": "2.0.0"}"#.to_vec(),
        ),
        (
            "Report/definition/report.json",
            br#"{"themeCollection": {}, "settings": {}}"#.to_vec(),
        ),
        (
            "Report/definition/pages/pages.json",
            br#"{"pageOrder": ["p1"], "activePageName": "p1"}"#.to_vec(),
        ),
        (
            "Report/definition/pages/p1/page.json",
            br#"{"name": "p1", "displayName": "Page 1", "displayOption": "FitToPage", "height": 720, "width": 1280}"#.to_vec(),
        ),
    ]
}

#[test]
fn thin_pbix_is_remote_and_connection_secrets_are_redacted() {
    let connections = json!({
        "Version": 1,
        "Connections": [{
            "Name": "EntityDataSource",
            "ConnectionString": "Data Source=pbiazure://api.powerbi.com;Initial Catalog=acme;Password=s3cr3t-pw",
            "ConnectionType": "pbiServiceLive",
            "PbiServiceModelId": 12345,
            "PbiModelVirtualServerName": "acme_virtualserver",
            "PbiModelDatabaseName": "00000000-0000-4000-8000-0000000000ac"
        }],
        "Extra": {"AccessToken": "s3cr3t-token", "Url": "https://a.blob.core.windows.net/c?sv=1&sig=s3cr3t-sig"}
    })
    .to_string();
    let mut files = pbir_report_files();
    files.push(("Version", utf16le("1.32")));
    files.push(("Connections", connections.into_bytes()));
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    let opened = open_bytes("Thin.pbix", zip_of(&refs), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbix);
    assert!(doc.model.is_none());
    assert_eq!(
        doc.model_status,
        ModelStatus::Remote {
            description: "remote, not available offline".into()
        }
    );
    match &doc.reports[0].dataset_reference {
        DatasetReference::ByConnection {
            connection_string,
            database_name,
            connection_type,
            model_id,
            raw,
            ..
        } => {
            assert_eq!(connection_string, &None);
            assert_eq!(
                database_name.as_deref(),
                Some("00000000-0000-4000-8000-0000000000ac")
            );
            assert_eq!(connection_type.as_deref(), Some("pbiServiceLive"));
            assert_eq!(model_id.as_deref(), Some("12345"));
            assert_eq!(raw["ConnectionString"], REDACTED);
        }
        other => panic!("unexpected dataset reference {other:?}"),
    }
    let part = opened.part("connections").unwrap();
    assert_eq!(
        part.redacted,
        [
            "/Connections/0/ConnectionString",
            "/Extra/AccessToken",
            "/Extra/Url"
        ]
    );
    assert!(has_diag(doc, codes::REDACTED));
    assert!(has_diag(doc, codes::REMOTE_MODEL));
    let everything = opened.inspect_json().to_string();
    assert!(
        !everything.contains("s3cr3t"),
        "secret leaked: {everything}"
    );

    // Saving keeps the live binding from the original Connections part.
    let dir = tmp_dir();
    let out = dir.path().join("thin");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    let definition: Value =
        serde_json::from_slice(&std::fs::read(out.join("Thin.Report/definition.pbir")).unwrap())
            .unwrap();
    assert_eq!(
        definition["datasetReference"]["byConnection"]["connectionType"],
        "pbiServiceLive"
    );
    let reopened = open(&out);
    assert!(reopened.document.model_status != ModelStatus::Local);
    assert!(reopened.document.reports[0].dataset_reference.is_remote());
}

#[test]
fn pbids_lists_connections_without_secrets() {
    let pbids = json!({
        "version": "0.1",
        "connections": [
            {
                "details": {"protocol": "tds", "address": {"server": "sql.contoso.com", "database": "Sales"}},
                "options": {},
                "mode": "DirectQuery"
            },
            {
                "details": {"protocol": "azure-blobs", "address": {"url": "https://acct.blob.core.windows.net/c?sv=1&sig=s3cr3t"}},
                "options": {}
            }
        ]
    })
    .to_string();
    let opened = open_bytes("Sales.pbids", pbids.into_bytes(), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbids);
    assert!(doc.model.is_none() && doc.reports.is_empty());
    assert_eq!(doc.model_status, ModelStatus::Missing);
    let infos: Vec<&str> = doc
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::PBIDS)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(infos.len(), 2);
    assert!(infos[0].contains("protocol `tds`") && infos[0].contains("server=sql.contoso.com"));
    assert!(infos[0].contains("DirectQuery"));
    assert!(infos[1].contains("protocol `azure-blobs`") && infos[1].contains("sig=<redacted>"));
    assert!(!opened.inspect_json().to_string().contains("s3cr3t"));
}

#[test]
fn bim_file_and_bim_semantic_model_folder() {
    let bim = sample_bim();
    let opened = open_bytes(
        "model.bim",
        bim.clone().into_bytes(),
        &OpenOptions::default(),
    )
    .unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Bim);
    assert_eq!(doc.model_status, ModelStatus::Local);
    let m = model(&opened);
    assert_eq!(m.tables[0].name, "Orders");
    assert_eq!(m.compatibility_level(), Some(1567));
    assert_eq!(m.tables[0].origin.as_ref().unwrap().path, "model.bim");
    assert!(
        matches!(&opened.model_source, Some(ModelSource::Tmsl { entry, .. }) if entry == "model.bim")
    );
    assert_ids_unique(doc);

    // A `*.SemanticModel` folder holding a `model.bim`, saved as TMDL.
    let dir = tmp_dir();
    let folder = dir.path().join("Sales.SemanticModel");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("model.bim"), &bim).unwrap();
    std::fs::write(folder.join("definition.pbism"), r#"{"version": "1.0"}"#).unwrap();
    let from_folder = open(&folder);
    assert_eq!(
        from_folder.document.source.format,
        SourceFormat::SemanticModelFolder
    );
    assert_eq!(
        normalized(&from_folder.document.model),
        normalized(&doc.model)
    );

    let out = dir.path().join("saved");
    save_as_pbip(&from_folder, &out, &SaveOptions::default()).unwrap();
    assert!(
        out.join("Sales.SemanticModel/definition/model.tmdl")
            .is_file()
    );
    assert!(!out.join("Sales.SemanticModel/model.bim").exists());
    let pbism: Value = serde_json::from_slice(
        &std::fs::read(out.join("Sales.SemanticModel/definition.pbism")).unwrap(),
    )
    .unwrap();
    assert_eq!(pbism["version"], "4.2", "TMDL needs a 4.x definition.pbism");
    let saved = open(&out.join("Sales.SemanticModel"));
    assert!(
        errors(&saved.document).is_empty(),
        "{:?}",
        errors(&saved.document)
    );
    // The multi-line `expression` string (instead of TOM's array of lines)
    // is TMSL layout kept as a shadow in `extra`; it is not written to TMDL.
    let partition = &model(&opened).tables[0].partitions[0];
    assert!(partition.extra.contains_key("source"));
    let mut expected = normalized(&doc.model);
    expected["tables"][0]["partitions"][0]
        .as_object_mut()
        .unwrap()
        .remove("extra");
    assert_eq!(normalized(&saved.document.model), expected);
}

/// A measure's dynamic format string is `{"expression": …}` in TOM JSON and
/// `formatStringDefinition = …` in TMDL; conversions keep each format's form.
#[test]
fn measure_format_string_definitions_convert_between_tmsl_and_tmdl() {
    let bim = json!({
        "compatibilityLevel": 1601,
        "model": {"tables": [{
            "name": "Orders",
            "measures": [{
                "name": "Sales (Scaled)",
                "expression": "[Total Sales]",
                "formatStringDefinition": {"expression": ["IF(", "    [Total Sales] >= 1000, \"#,0.0,K\", \"#,0\")"]}
            }]
        }]}
    })
    .to_string();
    let from_tmsl = pbi_tmsl::read_bim(&bim).unwrap().model;
    let files = pbi_tmdl::write_model(&from_tmsl, None);
    let (_, text) = files
        .iter()
        .find(|(p, _)| p == "tables/Orders.tmdl")
        .unwrap();
    assert!(
        text.contains(
            "\t\tformatStringDefinition =\n\t\t\t\tIF(\n\t\t\t\t    [Total Sales] >= 1000"
        ),
        "{text}"
    );
    let from_tmdl = pbi_tmdl::load_model(&files).unwrap().model;
    let measure = from_tmdl.measure("Orders", "Sales (Scaled)").unwrap();
    assert_eq!(
        measure.format_string_definition().as_deref(),
        Some("IF(\n    [Total Sales] >= 1000, \"#,0.0,K\", \"#,0\")")
    );
    let back: Value = serde_json::from_str(&pbi_tmsl::write_bim(&from_tmdl, None, None)).unwrap();
    assert_eq!(
        back["model"]["tables"][0]["measures"][0]["formatStringDefinition"],
        json!({"expression": ["IF(", "    [Total Sales] >= 1000, \"#,0.0,K\", \"#,0\")"]})
    );
}

#[test]
fn zip_slip_archives_are_refused() {
    let evil = zip_of(&[("Version", &utf16le("1.32")), ("../evil.txt", b"owned")]);
    let err = open_bytes("evil.pbix", evil, &OpenOptions::default()).unwrap_err();
    match err {
        CoreError::Container(ContainerError::UnsafePath { reason, .. }) => {
            assert_eq!(reason, UnsafePathReason::ParentTraversal);
        }
        other => panic!("unexpected error {other}"),
    }
}

#[test]
fn unknown_formats_are_unsupported_and_minimal_new_formats_open() {
    let err = open_bytes(
        "mystery.bin",
        vec![0x00, 0x13, 0x37, 0xfe, 0x01, 0x02, 0xff, 0x00, 0x10],
        &OpenOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::Unsupported { .. }), "{err}");

    let rdl = br#"<?xml version="1.0"?><Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition"><DataSets/></Report>"#;
    let opened = open_bytes("Paginated.rdl", rdl.to_vec(), &OpenOptions::default()).unwrap();
    assert_eq!(opened.document.source.format, SourceFormat::Rdl);
    assert!(!has_diag(&opened.document, codes::NOT_IMPLEMENTED));
    assert_eq!(
        opened.paginated_report().unwrap().schema.as_deref(),
        Some("2016/01")
    );
    assert!(opened.document.model.is_none());
    assert_eq!(opened.document.source.entries, ["Paginated.rdl"]);

    let vpax = zip_of(&[("DaxVpaView.json", b"{}"), ("DaxModel.json", b"{}")]);
    let opened = open_bytes("Model.vpax", vpax, &OpenOptions::default()).unwrap();
    assert_eq!(opened.document.source.format, SourceFormat::Vpax);
    assert!(!has_diag(&opened.document, codes::NOT_IMPLEMENTED));
    assert!(opened.vpax_stats().unwrap().tables.is_empty());
    assert!(opened.document.extra_parts.is_empty());
}

#[test]
fn invalid_tmdl_becomes_an_error_diagnostic() {
    let dir = tmp_dir();
    let folder = dir.path().join("Broken.SemanticModel");
    std::fs::create_dir_all(folder.join("definition")).unwrap();
    std::fs::write(folder.join("definition.pbism"), r#"{"version": "4.2"}"#).unwrap();
    std::fs::write(
        folder.join("definition/model.tmdl"),
        "model Model\n\t\t\tculture: en-US\n",
    )
    .unwrap();
    let opened = open(&folder);
    let doc = &opened.document;
    assert!(doc.model.is_none());
    assert_eq!(doc.model_status, ModelStatus::Missing);
    let error = doc
        .diagnostics
        .iter()
        .find(|d| d.code == codes::MODEL_FAILED)
        .expect("model error");
    assert_eq!(error.severity, Severity::Error);
    assert_eq!(
        error.origin.as_ref().map(|o| o.path.as_str()),
        Some("definition/model.tmdl")
    );
}

#[test]
fn extra_part_kinds() {
    assert_eq!(part_kind("SecurityBindings"), "securityBindings");
    assert_eq!(
        part_kind("Report/StaticResources/RegisteredResources/logo.png"),
        "image"
    );
    assert_eq!(
        part_kind("X.Report/StaticResources/SharedResources/BaseThemes/CY24SU10.json"),
        "theme"
    );
    assert_eq!(part_kind("X.SemanticModel/.pbi/cache.abf"), "cacheAbf");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for (rel, bytes) in files_under(from) {
        let path = to.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

#[test]
fn opened_documents_can_be_shared_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OpenedDocument>();
}

#[test]
fn lone_report_folders_save_as_complete_projects() {
    let dir = tmp_dir();

    // A report folder with its sibling model → project with both.
    let opened = open(&example(STORES));
    let out = dir.path().join("stores");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    assert!(
        out.join(format!("{MODEL_FOLDER}/definition/model.tmdl"))
            .is_file()
    );
    assert!(out.join(format!("{STORES}/definition.pbir")).is_file());
    let project = open(&out);
    assert_eq!(project.document.model_status, ModelStatus::Local);
    assert_acme_model(model(&project));
    assert_eq!(
        normalized(&project.document.reports),
        normalized(&opened.document.reports)
    );

    // A live (byConnection) report folder → report-only project.
    let opened = open(&remote_report_folder(dir.path()));
    let out = dir.path().join("live");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    let project = open(&out);
    assert!(project.document.model.is_none());
    assert!(matches!(
        project.document.model_status,
        ModelStatus::Remote { .. }
    ));
    assert_eq!(
        normalized(&project.document.reports[0].pages),
        normalized(&opened.document.reports[0].pages)
    );
}

#[test]
fn local_cache_and_settings_of_a_semantic_model() {
    let dir = tmp_dir();
    let folder = dir.path().join(MODEL_FOLDER);
    copy_dir(&example(MODEL_FOLDER), &folder);
    std::fs::create_dir_all(folder.join(".pbi")).unwrap();
    std::fs::write(folder.join(".pbi/cache.abf"), b"not really a backup").unwrap();
    std::fs::write(
        folder.join(".pbi/localSettings.json"),
        r#"{"version": "1.0", "securityBindingsSignature": "AQAAANCMnd8BFdERjHoAwE", "remoteModelingObjectId": "x"}"#,
    )
    .unwrap();

    let opened = open(&folder);
    let doc = &opened.document;
    assert_eq!(doc.model_status, ModelStatus::Local);
    let cache = doc
        .diagnostics
        .iter()
        .find(|d| d.code == codes::CACHE_ABF)
        .expect("cache note");
    assert_eq!(cache.severity, Severity::Warning);
    assert!(
        doc.extra_parts
            .iter()
            .any(|p| p.path == ".pbi/cache.abf" && p.kind == "cacheAbf")
    );
    let settings = opened.part("localSettings").expect("localSettings.json");
    assert_eq!(settings.json.as_ref().unwrap()["version"], "1.0");

    let quiet = OpenOptions {
        load_pbix_schema: false,
        ..OpenOptions::default()
    };
    let opened = open_path(&folder, &quiet).unwrap();
    let cache = opened
        .document
        .diagnostics
        .iter()
        .find(|d| d.code == codes::CACHE_ABF)
        .expect("cache note");
    assert_eq!(cache.severity, Severity::Info);

    // The cache and local settings are not part of a saved project.
    let out = dir.path().join("saved");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    assert!(!out.join(format!("{MODEL_FOLDER}/.pbi")).exists());
}

#[test]
fn multi_model_project_keeps_extra_models() {
    let dir = tmp_dir();
    let root = dir.path().join("multi");
    let model_src = example(MODEL_FOLDER);
    copy_dir(&model_src, &root.join("A.SemanticModel"));
    copy_dir(&model_src, &root.join("B.SemanticModel"));
    for report in ["One.Report", "Two.Report"] {
        copy_dir(&example(OVERVIEW), &root.join(report));
        std::fs::write(
            root.join(report).join("definition.pbir"),
            r#"{"version": "4.0", "datasetReference": {"byPath": {"path": "../B.SemanticModel"}}}"#,
        )
        .unwrap();
        std::fs::remove_file(root.join(report).join(".platform")).unwrap();
    }

    let opened = open(&root);
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbip);
    assert_eq!(
        opened.model_source.as_ref().unwrap().folder(),
        "B.SemanticModel"
    );
    assert_eq!(opened.extra_models.len(), 1);
    assert_eq!(opened.extra_models[0].source.folder(), "A.SemanticModel");
    assert!(has_diag(doc, codes::MULTIPLE_MODELS));
    let names: Vec<&str> = doc.reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["One", "Two"]);
    assert!(errors(doc).is_empty(), "{:?}", errors(doc));

    // IDs stay unique across the document and the extra models.
    let mut ids = HashSet::new();
    doc.walk(&mut |id, _| {
        ids.insert(id);
        ControlFlow::Continue(())
    });
    let extra = Document {
        model: Some(opened.extra_models[0].model.clone()),
        ..Document::default()
    };
    extra.walk(&mut |id, _| {
        assert!(id.is_assigned());
        assert!(ids.insert(id), "duplicate id {id:?}");
        ControlFlow::Continue(())
    });

    let out = dir.path().join("saved");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    assert!(out.join("A.SemanticModel/definition/model.tmdl").is_file());
    assert!(out.join("B.SemanticModel/definition/model.tmdl").is_file());
    assert!(out.join("multi.pbip").is_file());
    let again = open(&out);
    assert_eq!(again.extra_models.len(), 1);
    // Saving adds a `.platform` (display name, logicalId) to each report.
    for (a, b) in again.document.reports.iter().zip(&doc.reports) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.dataset_reference, b.dataset_reference);
        assert_eq!(normalized(&a.pages), normalized(&b.pages));
    }
    let platform: Value =
        serde_json::from_slice(&std::fs::read(out.join("One.Report/.platform")).unwrap()).unwrap();
    assert_eq!(platform["metadata"]["type"], "Report");
    assert_eq!(platform["metadata"]["displayName"], "One");
    assert_eq!(platform["config"]["logicalId"].as_str().unwrap().len(), 36);
}

#[test]
fn files_that_only_claim_a_container_format_are_refused() {
    let dir = tmp_dir();
    let fake = dir.path().join("fake.pbix");
    std::fs::write(&fake, b"this is plain text, not a ZIP archive").unwrap();
    let err = open_path(&fake, &OpenOptions::default()).unwrap_err();
    assert!(matches!(err, CoreError::Unsupported { .. }), "{err}");
}

#[test]
fn abf_backups_open_as_models() {
    let Some((pbix, facts)) = microsoft_sample("revenue-opportunities") else {
        return;
    };
    // The PBIX `DataModel` on its own: an XPress9-compressed backup.
    let dir = tmp_dir();
    let data_model = dir.path().join("DataModel");
    std::fs::write(&data_model, zip_entries(&pbix).remove("DataModel").unwrap()).unwrap();
    let compressed = open(&data_model);
    assert_eq!(compressed.document.source.format, SourceFormat::Abf);
    assert_eq!(compressed.document.model_status, ModelStatus::Local);
    let names: Vec<&str> = model(&compressed)
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    let expected: Vec<&str> = facts["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, expected);
    let Some(ModelSource::Abf {
        archive,
        compressed: true,
        ..
    }) = &compressed.model_source
    else {
        panic!("unexpected model source {:?}", compressed.model_source);
    };

    // The decompressed backup as an uncompressed `.abf`.
    let abf = dir.path().join("Model.abf");
    std::fs::write(&abf, archive.as_bytes()).unwrap();
    let plain = open(&abf);
    assert_eq!(plain.document.source.format, SourceFormat::Abf);
    assert!(matches!(
        plain.model_source,
        Some(ModelSource::Abf {
            compressed: false,
            ..
        })
    ));
    assert_eq!(
        normalized(&plain.document.model),
        normalized(&compressed.document.model)
    );
    assert!(
        errors(&plain.document).is_empty(),
        "{:?}",
        errors(&plain.document)
    );
}

#[test]
fn renamed_model_folders_are_rebound_on_save() {
    let dir = tmp_dir();
    let src = dir.path().join("src");
    copy_dir(&example(MODEL_FOLDER), &src.join("Model"));
    copy_dir(&example(OVERVIEW), &src.join("Sales.Report"));
    std::fs::write(
        src.join("Sales.Report/definition.pbir"),
        r#"{"version": "4.0", "datasetReference": {"byPath": {"path": "../Model"}}}"#,
    )
    .unwrap();
    let opened = open(&src.join("Sales.Report"));
    assert_eq!(opened.document.model_status, ModelStatus::Local);

    let out = dir.path().join("out");
    save_as_pbip(&opened, &out, &SaveOptions::default()).unwrap();
    let definition: Value =
        serde_json::from_slice(&std::fs::read(out.join("Sales.Report/definition.pbir")).unwrap())
            .unwrap();
    assert_eq!(
        definition["datasetReference"]["byPath"]["path"],
        "../Sales.SemanticModel"
    );
    let project = open(&out);
    assert_eq!(project.document.model_status, ModelStatus::Local);
    assert!(errors(&project.document).is_empty());
    assert!(!has_diag(&project.document, codes::MODEL_NOT_FOUND));
}
