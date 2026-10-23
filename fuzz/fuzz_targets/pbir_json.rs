//! PBIR JSON files: typed parse of single files and the report-folder
//! reader/writer (`pbi_pbir`).
//!
//! The input is one JSON file. It is (1) parsed as every PBIR file type, and
//! (2) placed into a small report folder — in the slot named by its
//! `$schema` URL, or one picked from its length — which is then read,
//! converted to the report AST and written back.
//!
//! Invariants: never panics; a typed parse re-serializes to identical JSON
//! (key order and number text included); `write_pbir` keeps unparsed files
//! byte for byte and parsed ones JSON-identical; reading the written folder
//! again gives the same report AST.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_pbir::json::first_difference;
use pbi_pbir::theme::ThemeJson;
use pbi_pbir::{
    DefinitionPbir, MemoryFileSource, PageJson, PagesJson, Platform, ReportJson, VersionJson,
    VisualContainerJson, read_pbir, to_report_ast, write_pbir,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

const THEME_PATH: &str = "StaticResources/SharedResources/BaseThemes/CY24SU10.json";

/// `(slot path, $schema marker, skeleton content)`.
const SLOTS: &[(&str, &str, &str)] = &[
    (
        "definition.pbir",
        "/definitionProperties/",
        r#"{"version":"4.0","datasetReference":{"byPath":{"path":"../M.SemanticModel"}}}"#,
    ),
    (
        ".platform",
        "/platformProperties/",
        r#"{"metadata":{"type":"Report","displayName":"R"},"config":{"version":"2.0","logicalId":"00000000-0000-0000-0000-000000000000"}}"#,
    ),
    (
        "definition/report.json",
        "/definition/report/",
        r#"{"themeCollection":{"baseTheme":{"name":"CY24SU10","reportVersionAtImport":"5.61","type":"SharedResources"}},"filterConfig":{"filters":[{"name":"f","field":{"Column":{"Expression":{"SourceRef":{"Entity":"T"}},"Property":"C"}},"type":"Categorical"}]}}"#,
    ),
    (
        "definition/version.json",
        "/versionMetadata/",
        r#"{"version":"2.0.0"}"#,
    ),
    (
        "definition/pages/pages.json",
        "/pagesMetadata/",
        r#"{"pageOrder":["p1"],"activePageName":"p1"}"#,
    ),
    (
        "definition/pages/p1/page.json",
        "/definition/page/",
        r#"{"name":"p1","displayName":"P","displayOption":"FitToPage","height":720,"width":1280}"#,
    ),
    (
        "definition/pages/p1/visuals/v1/visual.json",
        "/visualContainer/",
        r#"{"name":"v1","position":{"x":0,"y":0,"z":0,"width":100,"height":50},"visual":{"visualType":"card","query":{"queryState":{"Values":{"projections":[{"field":{"Measure":{"Expression":{"SourceRef":{"Entity":"T"}},"Property":"M"}},"queryRef":"T.M"}]}}}}}"#,
    ),
    (
        THEME_PATH,
        "\"dataColors\"",
        r##"{"name":"CY24SU10","dataColors":["#118DFF","#12239E"],"foreground":"#252423"}"##,
    ),
];

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

fn check_typed<T: DeserializeOwned + Serialize>(original: &Value) {
    let Ok(typed) = serde_json::from_value::<T>(original.clone()) else {
        return;
    };
    let back = match serde_json::to_value(&typed) {
        Ok(v) => v,
        Err(e) => panic!(
            "{}: typed value does not serialize: {e}",
            std::any::type_name::<T>()
        ),
    };
    if let Some(at) = first_difference(&back, original) {
        panic!(
            "{}: typed round-trip differs at {at}\noriginal: {original}\nwritten:  {back}",
            std::any::type_name::<T>()
        );
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(original) = serde_json::from_slice::<Value>(strip_bom(data)) {
        check_typed::<DefinitionPbir>(&original);
        check_typed::<Platform>(&original);
        check_typed::<ReportJson>(&original);
        check_typed::<VersionJson>(&original);
        check_typed::<PagesJson>(&original);
        check_typed::<PageJson>(&original);
        check_typed::<VisualContainerJson>(&original);
        check_typed::<ThemeJson>(&original);
    }

    let slot = SLOTS
        .iter()
        .position(|(_, marker, _)| contains(data, marker.as_bytes()))
        .unwrap_or(data.len() % SLOTS.len());
    let source: MemoryFileSource = SLOTS
        .iter()
        .enumerate()
        .map(|(i, (path, _, skeleton))| {
            let bytes = if i == slot {
                data.to_vec()
            } else {
                skeleton.as_bytes().to_vec()
            };
            (*path, bytes)
        })
        .collect();

    let report = match read_pbir(&source) {
        Ok(r) => r,
        Err(e) => panic!("the skeleton always has a PBIR definition: {e}"),
    };
    let (ast, diagnostics) = to_report_ast(&report);

    let written = write_pbir(&report);
    let paths: Vec<&String> = written.iter().map(|(p, _)| p).collect();
    let originals: Vec<&String> = source.files.keys().collect();
    assert_eq!(paths, originals, "write_pbir changed the file set");
    for (path, bytes) in &written {
        let original = &source.files[path];
        if report.files[path].parsed {
            let (Ok(a), Ok(b)) = (
                serde_json::from_slice::<Value>(strip_bom(bytes)),
                serde_json::from_slice::<Value>(strip_bom(original)),
            ) else {
                panic!("{path}: a parsed file is not JSON after writing");
            };
            if let Some(at) = first_difference(&a, &b) {
                panic!("{path}: written JSON differs at {at}\noriginal: {b}\nwritten:  {a}");
            }
        } else {
            assert!(
                bytes == original,
                "{path}: unparsed file was not kept verbatim"
            );
        }
    }

    let rewritten: MemoryFileSource = written.into_iter().collect();
    let again = match read_pbir(&rewritten) {
        Ok(r) => r,
        Err(e) => panic!("written folder does not read back: {e}"),
    };
    let (ast_again, diagnostics_again) = to_report_ast(&again);
    assert!(ast == ast_again, "report AST changed through write_pbir");
    assert_eq!(diagnostics.len(), diagnostics_again.len());
});
