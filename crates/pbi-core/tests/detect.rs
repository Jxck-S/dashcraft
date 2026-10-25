//! Content-based detection over the committed Acme Outfitters example, the
//! Microsoft sample files and synthetic samples.
//!
//! Fixtures are found through `$DASHCRAFT_REFERENCE`, falling back to the
//! repository's `reference/` folder. The fetched (gitignored) samples are
//! skipped with a `SKIP:` message when missing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use pbi_core::container::{Limits, TextEncoding};
use pbi_core::detect::{
    Confidence, DatasetReference, DetectError, DetectedFormat, Detection, ModelFlavour, PbixReport,
    ReportFlavour, detect_bytes, detect_path,
};
use pretty_assertions::assert_eq;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

/// A path inside the committed Acme Outfitters example.
fn example(rel: &str) -> PathBuf {
    reference_root().join("examples/acme-outfitters").join(rel)
}

/// The entries of `samples/microsoft/expected.json` whose file is present,
/// as `(file path, entry)`; missing files (the fetched ones) print SKIP.
fn microsoft_samples() -> Vec<(PathBuf, serde_json::Value)> {
    let path = reference_root().join("samples/microsoft/expected.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: {} missing", path.display());
        return Vec::new();
    };
    let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
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

/// A Microsoft sample by its `expected.json` id, if present.
fn microsoft_sample(id: &str) -> Option<PathBuf> {
    let path = reference_root().join("samples/microsoft/expected.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: {} missing", path.display());
        return None;
    };
    let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
    let entry = expected["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no sample {id} in expected.json"));
    let file = reference_root().join(entry["file"].as_str().unwrap());
    if file.is_file() {
        Some(file)
    } else {
        eprintln!("SKIP: {} missing", file.display());
        None
    }
}

/// Unzips `pbix` into a scratch folder (an "unzipped PBIX" layout).
fn unzip_to(pbix: &Path, dir: &Path) {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(pbix).unwrap()).unwrap();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let rel = entry.enclosed_name().unwrap();
        let out = dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).unwrap();
            continue;
        }
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&out).unwrap();
        std::io::copy(&mut entry, &mut file).unwrap();
    }
}

fn detect(path: &Path) -> Detection {
    detect_path(path, &Limits::default())
        .unwrap_or_else(|e| panic!("detect {}: {e}", path.display()))
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

fn has_signal(d: &Detection, needle: &str) -> bool {
    d.signals.iter().any(|s| s.contains(needle))
}

const XPRESS9_HEADER: &str = "This backup was created using XPress9 compression.";

// ---------------------------------------------------------------------------
// Committed fixtures
// ---------------------------------------------------------------------------

#[test]
fn pbip_project_folder() {
    let d = detect(&example(""));
    assert_eq!(d.format, DetectedFormat::Pbip);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(d.parts.pbip_file.as_deref(), Some("AcmeOutfitters.pbip"));
    assert_eq!(
        d.parts.pbip_artifacts,
        ["Acme Outfitters - Overview.Report"]
    );

    assert_eq!(d.parts.semantic_models.len(), 1);
    let model = &d.parts.semantic_models[0];
    assert_eq!(model.path, "AcmeOutfitters.SemanticModel");
    assert_eq!(model.flavour, ModelFlavour::Tmdl);
    assert!(!model.has_cache_abf);
    let platform = model.platform.as_ref().unwrap();
    assert_eq!(platform.item_type.as_deref(), Some("SemanticModel"));
    assert_eq!(platform.display_name.as_deref(), Some("AcmeOutfitters"));

    let mut reports: Vec<&str> = d.parts.reports.iter().map(|r| r.path.as_str()).collect();
    reports.sort_unstable();
    assert_eq!(
        reports,
        [
            "Acme Outfitters - Online.Report",
            "Acme Outfitters - Overview.Report",
            "Acme Outfitters - Stores.Report"
        ]
    );
    for r in &d.parts.reports {
        assert_eq!(r.flavour, ReportFlavour::Pbir, "{}", r.path);
        assert_eq!(
            r.dataset,
            DatasetReference::ByPath {
                path: "../AcmeOutfitters.SemanticModel".into(),
                resolved: Some("AcmeOutfitters.SemanticModel".into()),
            },
            "{}",
            r.path
        );
    }
    assert!(has_signal(&d, "AcmeOutfitters.pbip"));
    let json = serde_json::to_value(&d).unwrap();
    assert_eq!(json["format"]["kind"], "pbip");
}

#[test]
fn pbip_file_scans_its_project_folder() {
    let d = detect(&example("AcmeOutfitters.pbip"));
    assert_eq!(d.format, DetectedFormat::Pbip);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(d.parts.pbip_file.as_deref(), Some("AcmeOutfitters.pbip"));
    assert_eq!(d.parts.semantic_models.len(), 1);
    assert_eq!(d.parts.reports.len(), 3);
    assert!(d.parts.base_dir.is_some());
}

#[test]
fn semantic_model_folder() {
    let d = detect(&example("AcmeOutfitters.SemanticModel"));
    assert_eq!(d.format, DetectedFormat::PbipSemanticModelFolder);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(d.parts.semantic_models.len(), 1);
    assert_eq!(d.parts.semantic_models[0].path, "");
    assert_eq!(d.parts.semantic_models[0].flavour, ModelFlavour::Tmdl);
}

#[test]
fn report_folders() {
    let d = detect(&example("Acme Outfitters - Stores.Report"));
    assert_eq!(d.format, DetectedFormat::PbipReportFolder);
    assert_eq!(d.confidence, Confidence::Certain);
    let r = &d.parts.reports[0];
    assert_eq!(r.flavour, ReportFlavour::Pbir);
    assert_eq!(
        r.dataset,
        DatasetReference::ByPath {
            path: "../AcmeOutfitters.SemanticModel".into(),
            resolved: None,
        }
    );

    // The same report bound to a remote semantic model.
    let tmp = tempdir();
    let live = tmp.path().join("Acme Outfitters - Live.Report");
    let source = example("Acme Outfitters - Stores.Report");
    for rel in [
        ".platform",
        "definition/report.json",
        "definition/version.json",
    ] {
        let to = live.join(rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(source.join(rel), to).unwrap();
    }
    std::fs::write(
        live.join("definition.pbir"),
        br#"{"$schema":"https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json","version":"4.0","datasetReference":{"byConnection":{"connectionString":"Data Source=\"powerbi://api.powerbi.com/v1.0/myorg/Acme Outfitters\";initial catalog=AcmeOutfitters;semanticmodelid=00000000-0000-4000-8000-0000000000ac"}}}"#,
    )
    .unwrap();
    let live = detect(&live);
    assert_eq!(live.format, DetectedFormat::PbipReportFolder);
    assert_eq!(
        live.parts.reports[0].dataset,
        DatasetReference::ByConnection
    );
    assert!(has_signal(&live, "not available offline"));
}

#[test]
fn standalone_tmdl_folder_and_file() {
    let d = detect(&example("AcmeOutfitters.SemanticModel/definition"));
    assert_eq!(d.format, DetectedFormat::TmdlFolder);
    assert_eq!(d.confidence, Confidence::Certain);

    for file in ["model.tmdl", "tables/Orders.tmdl", "relationships.tmdl"] {
        let f = detect(&example(&format!(
            "AcmeOutfitters.SemanticModel/definition/{file}"
        )));
        assert_eq!(f.format, DetectedFormat::TmdlFile, "{file}");
        assert_eq!(f.confidence, Confidence::Certain, "{file}: {:?}", f.signals);
    }
}

#[test]
fn pbir_visual_json_file() {
    let pages = example("Acme Outfitters - Overview.Report/definition/pages");
    let visual = walk_find(&pages, "visual.json").expect("a visual.json fixture");
    let d = detect(&visual);
    assert_eq!(
        d.format,
        DetectedFormat::PbirJsonFile {
            schema_kind: "visualContainer".into()
        }
    );
    assert_eq!(d.confidence, Confidence::Certain);
    assert!(has_signal(&d, "visualContainer"));

    let page = walk_find(&pages, "page.json").unwrap();
    assert_eq!(
        detect(&page).format,
        DetectedFormat::PbirJsonFile {
            schema_kind: "page".into()
        }
    );
    let platform = detect(&example("Acme Outfitters - Overview.Report/.platform"));
    assert_eq!(
        platform.format,
        DetectedFormat::FabricJsonFile {
            item: "gitIntegration".into(),
            schema_kind: "platformProperties".into()
        }
    );
}

fn walk_find(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d).ok()?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if e.file_name() == name {
                return Some(p);
            }
        }
    }
    None
}

/// Every Microsoft sample is detected as recorded in `expected.json`.
#[test]
fn microsoft_samples_match_expected_json() {
    let samples = microsoft_samples();
    for (path, expected) in &samples {
        let d = detect(path);
        let facts = &expected["detect"];
        let json = serde_json::to_value(&d).unwrap();
        let id = &expected["id"];
        assert_eq!(json["format"]["kind"], facts["format"], "{id}");
        assert_eq!(d.format.label(), facts["label"].as_str().unwrap(), "{id}");
        assert_eq!(d.confidence, Confidence::Certain, "{id}");
        assert_eq!(d.parts.reports.len(), 1, "{id}");
        assert_eq!(d.parts.reports[0].path, "Report", "{id}");
        let flavour = match facts["reportFlavour"].as_str().unwrap() {
            "pbir" => ReportFlavour::Pbir,
            "legacyLayout" => ReportFlavour::LegacyLayout,
            other => panic!("{id}: unexpected flavour {other}"),
        };
        assert_eq!(d.parts.reports[0].flavour, flavour, "{id}");
    }
    let committed = samples.iter().filter(|(_, f)| f["committed"] == true);
    if reference_root().join("samples/microsoft").is_dir() {
        assert_eq!(committed.count(), 4);
    }
    if let Some(path) = microsoft_sample("revenue-opportunities") {
        assert_eq!(
            detect(&path).format,
            DetectedFormat::Pbix {
                has_data_model: true,
                report: PbixReport::Pbir
            }
        );
    }
    if let Some(path) = microsoft_sample("regional-sales") {
        assert_eq!(
            detect(&path).format,
            DetectedFormat::Pbix {
                has_data_model: true,
                report: PbixReport::LegacyLayout
            }
        );
    }
    if let Some(path) = microsoft_sample("covid-19-us-tracking") {
        assert_eq!(detect(&path).format, DetectedFormat::Pbit);
    }
}

#[test]
fn unzipped_pbix_folder_and_its_data_model() {
    let Some(pbix) = microsoft_sample("revenue-opportunities") else {
        return;
    };
    let tmp = tempdir();
    let dir = tmp.path().join("Revenue Opportunities");
    unzip_to(&pbix, &dir);
    let d = detect(&dir);
    assert_eq!(
        d.format,
        DetectedFormat::Pbix {
            has_data_model: true,
            report: PbixReport::Pbir
        }
    );
    assert!(has_signal(&d, "unzipped PBIX"));

    let model = detect(&dir.join("DataModel"));
    assert_eq!(model.format, DetectedFormat::Abf { compressed: true });
    assert_eq!(model.confidence, Confidence::Certain);

    let visual = walk_find(&dir.join("Report/definition/pages"), "visual.json").unwrap();
    assert_eq!(
        detect(&visual).format,
        DetectedFormat::PbirJsonFile {
            schema_kind: "visualContainer".into()
        }
    );
}

#[test]
fn pbit_data_model_schema_and_layout_entries() {
    let Some(pbit) = microsoft_sample("covid-19-us-tracking") else {
        return;
    };
    let tmp = tempdir();
    let dir = tmp.path().join("covid");
    unzip_to(&pbit, &dir);
    let schema = detect(&dir.join("DataModelSchema"));
    assert_eq!(schema.format, DetectedFormat::Bim);
    assert_eq!(schema.parts.text_encoding, Some(TextEncoding::Utf16Le));
    let layout = detect(&dir.join("Report/Layout"));
    assert_eq!(layout.format, DetectedFormat::LegacyLayoutJson);
    assert_eq!(layout.confidence, Confidence::Certain);
}

// ---------------------------------------------------------------------------
// Synthetic ZIP packages
// ---------------------------------------------------------------------------

fn pbit_bytes() -> Vec<u8> {
    let schema = utf16le(r#"{"name":"m","compatibilityLevel":1567,"model":{"tables":[]}}"#);
    let layout = utf16le(r#"{"id":0,"sections":[]}"#);
    zip_of(&[
        ("Version", &utf16le("1.28")),
        ("[Content_Types].xml", b"<Types/>"),
        ("DataModelSchema", &schema),
        ("Report/Layout", &layout),
    ])
}

#[test]
fn pbit_by_content() {
    let d = detect_bytes(Some("template.pbit"), &pbit_bytes());
    assert_eq!(d.format, DetectedFormat::Pbit);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(d.parts.reports[0].flavour, ReportFlavour::LegacyLayout);
    assert!(!has_signal(&d, "suggests"));
}

#[test]
fn wrong_extension_is_reported_and_content_wins() {
    let d = detect_bytes(Some("mislabelled.pbix"), &pbit_bytes());
    assert_eq!(d.format, DetectedFormat::Pbit);
    assert_eq!(d.confidence, Confidence::Certain);
    assert!(
        has_signal(&d, "extension `.pbix` suggests"),
        "{:?}",
        d.signals
    );

    let bim = br#"{"compatibilityLevel":1567,"model":{}}"#;
    let d = detect_bytes(Some("model.json.pbix"), bim);
    assert_eq!(d.format, DetectedFormat::Bim);
    assert!(has_signal(&d, "using the content"));
}

#[test]
fn legacy_layout_pbix() {
    let mut datamodel = utf16le(XPRESS9_HEADER);
    datamodel.extend([0, 0, 1, 2, 3]);
    let bytes = zip_of(&[
        ("Version", &utf16le("1.32")),
        ("DataModel", &datamodel),
        ("Report/Layout", &utf16le(r#"{"sections":[]}"#)),
        ("Connections", b"{}"),
    ]);
    let d = detect_bytes(Some("legacy.pbix"), &bytes);
    assert_eq!(
        d.format,
        DetectedFormat::Pbix {
            has_data_model: true,
            report: PbixReport::LegacyLayout
        }
    );
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn thin_report_pbix() {
    let bytes = zip_of(&[
        ("Version", &utf16le("1.32")),
        ("Connections", br#"{"Version":3}"#),
        ("Report/definition/report.json", b"{}"),
    ]);
    let d = detect_bytes(None, &bytes);
    assert_eq!(
        d.format,
        DetectedFormat::Pbix {
            has_data_model: false,
            report: PbixReport::Pbir
        }
    );
    assert!(has_signal(&d, "thin report"));
}

#[test]
fn excel_workbooks() {
    let with_model = zip_of(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("xl/workbook.xml", b"<workbook/>"),
        ("xl/model/item.data", b"abf"),
    ]);
    assert_eq!(
        detect_bytes(Some("book.xlsx"), &with_model).format,
        DetectedFormat::ExcelPowerPivot
    );
    let plain = zip_of(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("xl/workbook.xml", b"<workbook/>"),
    ]);
    let d = detect_bytes(Some("book.xlsm"), &plain);
    assert_eq!(d.format, DetectedFormat::ExcelNoModel);
    assert!(!has_signal(&d, "suggests"));
}

#[test]
fn vpax_pbiviz_pqt_and_connector() {
    let vpax = zip_of(&[
        ("DaxModel.json", b"{}"),
        ("DaxVpaView.json", b"{}"),
        ("Model.bim", b"{}"),
    ]);
    assert_eq!(detect_bytes(None, &vpax).format, DetectedFormat::Vpax);

    let pbiviz = zip_of(&[
        ("package.json", br#"{"visual":{}}"#),
        ("resources/abc123.pbiviz.json", b"{}"),
    ]);
    let d = detect_bytes(Some("visual.pbiviz"), &pbiviz);
    assert_eq!(d.format, DetectedFormat::Pbiviz);
    assert_eq!(d.confidence, Confidence::Certain);

    let pqt = zip_of(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("MashupDocument.pq", b"section Section1;"),
        ("MashupMetadata.json", b"{}"),
        ("Metadata.json", b"{}"),
    ]);
    assert_eq!(
        detect_bytes(Some("t.pqt"), &pqt).format,
        DetectedFormat::PowerQueryTemplate
    );
    let mashup = zip_of(&[
        ("Config/Package.xml", b"<Package/>"),
        ("Formulas/Section1.m", b"section Section1;"),
    ]);
    let d = detect_bytes(None, &mashup);
    assert_eq!(d.format, DetectedFormat::PowerQueryTemplate);
    assert_eq!(d.confidence, Confidence::Likely);

    let mez = zip_of(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("MyConnector.m", b"section MyConnector;"),
        ("resources.resx", b"<root/>"),
    ]);
    let d = detect_bytes(Some("MyConnector.mez"), &mez);
    assert_eq!(d.format, DetectedFormat::CustomConnector);
    assert_eq!(d.confidence, Confidence::Certain);

    let opaque = zip_of(&[("blob.bin", b"x")]);
    let d = detect_bytes(Some("signed.pqx"), &opaque);
    assert_eq!(d.format, DetectedFormat::CustomConnector);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn hostile_zip_is_refused() {
    let bytes = zip_of(&[("../../evil.txt", b"x"), ("DataModel", b"x")]);
    let d = detect_bytes(Some("evil.pbix"), &bytes);
    assert_eq!(
        d.format,
        DetectedFormat::Pbix {
            has_data_model: false,
            report: PbixReport::None
        }
    );
    assert_eq!(d.confidence, Confidence::ExtensionOnly);
    assert!(has_signal(&d, "refused"));

    let tmp = tempdir();
    let path = tmp.path().join("evil.pbix");
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        detect_path(&path, &Limits::default()),
        Err(DetectError::Container(_))
    ));
}

// ---------------------------------------------------------------------------
// Synthetic text and binary files
// ---------------------------------------------------------------------------

#[test]
fn bim_json_in_all_encodings() {
    let json = r#"{"name":"SemanticModel","compatibilityLevel":1567,"model":{"culture":"en-US","tables":[{"name":"T","columns":[{"name":"c","dataType":"int64"}]}]}}"#;
    let d = detect_bytes(Some("model.bim"), json.as_bytes());
    assert_eq!(d.format, DetectedFormat::Bim);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(d.parts.text_encoding, Some(TextEncoding::Utf8));

    let d = detect_bytes(Some("DataModelSchema"), &utf16le(json));
    assert_eq!(d.format, DetectedFormat::Bim);
    assert_eq!(d.parts.text_encoding, Some(TextEncoding::Utf16Le));
    assert!(has_signal(&d, "UTF-16LE"));

    let mut bom = vec![0xFF, 0xFE];
    bom.extend(utf16le(json));
    assert_eq!(detect_bytes(None, &bom).format, DetectedFormat::Bim);
    let mut bom8 = b"\xEF\xBB\xBF".to_vec();
    bom8.extend(json.as_bytes());
    assert_eq!(detect_bytes(None, &bom8).format, DetectedFormat::Bim);
}

#[test]
fn truncated_json_falls_back_to_key_heuristics() {
    let d = detect_bytes(
        None,
        br#"{"compatibilityLevel": 1567, "model": {"tables": ["#,
    );
    assert_eq!(d.format, DetectedFormat::Bim);
    assert_eq!(d.confidence, Confidence::Likely);
    let d = detect_bytes(
        None,
        br#"{"$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.1.0/schema.json", "name": "#,
    );
    assert_eq!(
        d.format,
        DetectedFormat::PbirJsonFile {
            schema_kind: "page".into()
        }
    );
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn json_documents() {
    let pbids = br#"{"version":"0.1","connections":[{"details":{"protocol":"tds","address":{"server":"srv","database":"db"}},"mode":"DirectQuery"}]}"#;
    assert_eq!(
        detect_bytes(Some("c.pbids"), pbids).format,
        DetectedFormat::Pbids
    );

    let pbip = br#"{"version":"1.0","artifacts":[{"report":{"path":"X.Report"}}],"settings":{}}"#;
    let d = detect_bytes(Some("X.pbip"), pbip);
    assert_eq!(d.format, DetectedFormat::Pbip);
    assert_eq!(d.parts.pbip_artifacts, ["X.Report"]);

    let dataflow = br#"{"name":"df","version":"1.0","pbi:mashup":{"document":"section Section1;"},"entities":[{"$type":"LocalEntity","name":"E"}]}"#;
    assert_eq!(
        detect_bytes(Some("model.json"), dataflow).format,
        DetectedFormat::DataflowModelJson
    );

    let pbir = br#"{"version":"4.0","datasetReference":{"byPath":{"path":"../M.SemanticModel"}}}"#;
    assert_eq!(
        detect_bytes(Some("definition.pbir"), pbir).format,
        DetectedFormat::PbirJsonFile {
            schema_kind: "definitionProperties".into()
        }
    );

    let d = detect_bytes(
        Some("definition.pbism"),
        br#"{"version":"4.2","settings":{}}"#,
    );
    assert_eq!(
        d.format,
        DetectedFormat::FabricJsonFile {
            item: "semanticModel".into(),
            schema_kind: "definitionProperties".into()
        }
    );
    assert_eq!(d.confidence, Confidence::Certain);

    let layout = utf16le(
        r#"{"id":0,"sections":[{"name":"ReportSection","visualContainers":[{"config":"{}"}]}],"config":"{}"}"#,
    );
    let d = detect_bytes(Some("Layout"), &layout);
    assert_eq!(d.format, DetectedFormat::LegacyLayoutJson);
    assert_eq!(d.confidence, Confidence::Certain);

    let theme = br##"{"name":"CY24SU10","dataColors":["#118DFF"]}"##;
    let d = detect_bytes(Some("theme.json"), theme);
    assert_eq!(d.format, DetectedFormat::Unknown);
    assert!(has_signal(&d, "keys: name, dataColors"));
}

#[test]
fn rdl_xml() {
    let rdl = br#"<?xml version="1.0" encoding="utf-8"?>
<!-- generated -->
<Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition" xmlns:rd="http://schemas.microsoft.com/SQLServer/reporting/reportdesigner">
  <DataSources />
</Report>"#;
    let d = detect_bytes(Some("sales.rdl"), rdl);
    assert_eq!(d.format, DetectedFormat::Rdl);
    assert_eq!(d.confidence, Confidence::Certain);
    let other = detect_bytes(None, b"<?xml version=\"1.0\"?><Types/>");
    assert_eq!(other.format, DetectedFormat::Unknown);
    assert!(has_signal(&other, "<Types>"));
}

#[test]
fn m_scripts() {
    let section =
        b"// queries\nsection Section1;\n\nshared Query1 = let\n    Source = 1\nin\n    Source;\n";
    let d = detect_bytes(None, section);
    assert_eq!(d.format, DetectedFormat::PowerQueryScript);
    assert_eq!(d.confidence, Confidence::Certain);

    let expr = b"let\n    Source = Csv.Document(File.Contents(\"x.csv\"))\nin\n    Source";
    let d = detect_bytes(None, expr);
    assert_eq!(d.format, DetectedFormat::PowerQueryScript);
    assert_eq!(d.confidence, Confidence::Likely);
    let d = detect_bytes(Some("query.pq"), expr);
    assert_eq!(d.confidence, Confidence::Certain);

    let sdk = b"// connector\n[Version = \"1.0.0\"]\nsection MyConnector;\n";
    assert_eq!(
        detect_bytes(None, sdk).format,
        DetectedFormat::PowerQueryScript
    );
}

#[test]
fn tmdl_text() {
    let tmdl =
        b"/// Stores and online channels\ntable Stores\n\tlineageTag: 1\n\n\tcolumn 'Store ID'\n";
    let d = detect_bytes(None, tmdl);
    assert_eq!(d.format, DetectedFormat::TmdlFile);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn abf_signatures() {
    let mut single = utf16le(XPRESS9_HEADER);
    single.extend([0, 0, 0xF0, 0x18]);
    let d = detect_bytes(None, &single);
    assert_eq!(d.format, DetectedFormat::Abf { compressed: true });
    assert_eq!(d.confidence, Confidence::Certain);

    let mut multi = utf16le("This backup was created using multithreaded XPrs9.");
    multi.extend([0, 0, 1, 2]);
    assert_eq!(
        detect_bytes(None, &multi).format,
        DetectedFormat::Abf { compressed: true }
    );

    let mut raw = vec![0xFF, 0xFE];
    raw.extend(utf16le("STREAM_STORAGE_SIGNATURE_)!@#$%^&*("));
    raw.extend([0u8; 64]);
    let d = detect_bytes(Some("cache.abf"), &raw);
    assert_eq!(d.format, DetectedFormat::Abf { compressed: false });
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn extension_only_and_unknown() {
    let binary = [0u8, 1, 2, 3, 0xFF, 0xFE, 0x10, 0x00, 0x7F, 0x80];
    let d = detect_bytes(Some("report.rdl"), &binary);
    assert_eq!(d.format, DetectedFormat::Rdl);
    assert_eq!(d.confidence, Confidence::ExtensionOnly);
    assert!(has_signal(&d, "extension"));

    let d = detect_bytes(None, &binary);
    assert_eq!(d.format, DetectedFormat::Unknown);
    assert_eq!(d.confidence, Confidence::NoSignal);
    assert_eq!(detect_bytes(None, b"").format, DetectedFormat::Unknown);
    assert_eq!(
        detect_bytes(Some("notes.txt"), b"hello world").format,
        DetectedFormat::Unknown
    );
}

#[test]
fn folder_layouts_built_on_disk() {
    let tmp = tempdir();
    let root = tmp.path();
    let write = |rel: &str, data: &[u8]| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    };
    // Fabric Git export: item folders with `.platform`, no `.pbip`; a
    // legacy `model.bim` model with a local cache, and a PBIR-Legacy report.
    write(
        "Sales.SemanticModel/.platform",
        br#"{"metadata":{"type":"SemanticModel","displayName":"Sales"},"config":{"logicalId":"1"}}"#,
    );
    write(
        "Sales.SemanticModel/definition.pbism",
        br#"{"version":"1.0"}"#,
    );
    write("Sales.SemanticModel/model.bim", b"{}");
    write("Sales.SemanticModel/.pbi/cache.abf", b"abf");
    write(
        "Sales.Report/definition.pbir",
        br#"{"version":"1.0","datasetReference":{"byPath":{"path":"../Sales.SemanticModel"}}}"#,
    );
    write("Sales.Report/report.json", br#"{"sections":[]}"#);
    write(
        "Flow.Dataflow/.platform",
        br#"{"metadata":{"type":"Dataflow"},"config":{}}"#,
    );
    write("Flow.Dataflow/mashup.pq", b"section Section1;");

    let d = detect(root);
    assert_eq!(d.format, DetectedFormat::Pbip);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(has_signal(&d, "no `.pbip` file"));
    let model = &d.parts.semantic_models[0];
    assert_eq!(model.flavour, ModelFlavour::Bim);
    assert!(model.has_cache_abf);
    assert!(has_signal(&d, "cache.abf"));
    assert_eq!(d.parts.reports[0].flavour, ReportFlavour::PbirLegacy);
    assert_eq!(d.parts.other_items.len(), 1);
    assert_eq!(d.parts.other_items[0].item_type, "Dataflow");

    let flow = detect(&root.join("Flow.Dataflow"));
    assert_eq!(flow.format, DetectedFormat::DataflowFolder);
    assert_eq!(flow.confidence, Confidence::Certain);

    // A model folder identified only by name and `.platform`.
    write(
        "Other/.platform",
        br#"{"metadata":{"type":"Notebook"},"config":{}}"#,
    );
    assert_eq!(
        detect(&root.join("Other")).format,
        DetectedFormat::FabricItemFolder {
            item_type: "Notebook".into()
        }
    );

    // Without a `.pbip`, only conventionally named item folders count: a
    // folder that merely contains some report folder is not a project.
    write(
        "loose/live/definition.pbir",
        br#"{"version":"4.0","datasetReference":{"byConnection":{}}}"#,
    );
    write(
        "loose/live/.platform",
        br#"{"metadata":{"type":"Report"},"config":{}}"#,
    );
    write("loose/notes.txt", b"x");
    let loose = detect(&root.join("loose"));
    assert_eq!(loose.format, DetectedFormat::Unknown);
    assert_eq!(
        detect(&root.join("loose/live")).format,
        DetectedFormat::PbipReportFolder
    );
}

#[test]
fn detection_never_panics_on_prefixes_and_mutations() {
    let mut samples: Vec<Vec<u8>> = vec![
        pbit_bytes(),
        utf16le(r#"{"compatibilityLevel":1,"model":{"tables":[]}}"#),
        b"<?xml version=\"1.0\"?><Report xmlns=\"reportdefinition\"/>".to_vec(),
        b"section S; shared Q = 1;".to_vec(),
        br#"{"$schema":"https://x/fabric/item/report/definition/page/1/schema.json"}"#.to_vec(),
    ];
    let mut abf = vec![0xFF, 0xFE];
    abf.extend(utf16le("STREAM_STORAGE_SIGNATURE_)!@#$%^&*("));
    samples.push(abf);
    for s in &samples {
        for cut in 0..s.len().min(600) {
            let _ = detect_bytes(Some("x.pbix"), &s[..cut]);
        }
        for i in (0..s.len()).step_by(7) {
            let mut m = s.clone();
            m[i] ^= 0x5A;
            let _ = detect_bytes(None, &m);
        }
    }
}

/// A temporary directory under Cargo's per-target scratch folder (keeps
/// test files inside the build tree).
fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbi-core-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}
