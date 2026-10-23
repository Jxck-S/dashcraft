#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Lossless round-trip of every PBIR file of the Acme Outfitters example
//! reports (the PBIR inside the Microsoft sample PBIX files is checked by
//! `pbi-core`'s `readers.rs`, which opens the packages).

mod common;

use common::{DirSource, report_folders};
use pbi_pbir::json::first_difference;
use pbi_pbir::theme::ThemeJson;
use pbi_pbir::{
    DefinitionPbir, PageJson, PagesJson, PbirFileKind, Platform, ReportJson, VersionJson,
    VisualContainerJson, read_pbir, write_pbir,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn retyped<T: DeserializeOwned + Serialize>(original: &Value) -> Result<Value, String> {
    let typed: T = serde_json::from_value(original.clone()).map_err(|e| e.to_string())?;
    serde_json::to_value(&typed).map_err(|e| e.to_string())
}

fn is_json_path(path: &str) -> bool {
    path.ends_with(".json") || path.ends_with(".pbir") || path.ends_with(".platform")
}

/// Every JSON file under every fixture report folder: typed parse +
/// re-serialize gives identical JSON (key order and number text included).
#[test]
fn every_fixture_json_file_round_trips_through_its_typed_struct() {
    let (mut checked, mut bookmarks) = (0, 0);
    for (label, folder) in report_folders() {
        let source = DirSource(folder.clone());
        let mut paths = pbi_pbir::FileSource::list(&source, "");
        paths.sort();
        for path in paths.into_iter().filter(|p| is_json_path(p)) {
            let bytes = std::fs::read(folder.join(&path)).unwrap();
            let original: Value = serde_json::from_slice(&bytes).unwrap();
            let back = match PbirFileKind::of(&path) {
                PbirFileKind::DefinitionPbir => retyped::<DefinitionPbir>(&original),
                PbirFileKind::Platform => retyped::<Platform>(&original),
                PbirFileKind::Report => retyped::<ReportJson>(&original),
                PbirFileKind::Version => retyped::<VersionJson>(&original),
                PbirFileKind::Pages => retyped::<PagesJson>(&original),
                PbirFileKind::Page { .. } => retyped::<PageJson>(&original),
                PbirFileKind::Visual { .. } => retyped::<VisualContainerJson>(&original),
                PbirFileKind::Other if path.starts_with("StaticResources/") => {
                    retyped::<ThemeJson>(&original)
                }
                // Bookmarks are not typed; they are kept byte for byte (checked below).
                PbirFileKind::Other if path.starts_with("definition/bookmarks/") => {
                    bookmarks += 1;
                    continue;
                }
                PbirFileKind::Other => panic!("{label}/{path}: unexpected JSON file"),
            }
            .unwrap_or_else(|e| panic!("{label}/{path}: {e}"));
            assert_eq!(back, original, "{label}/{path}: values differ");
            assert_eq!(
                first_difference(&back, &original),
                None,
                "{label}/{path}: key order or number text differs"
            );
            checked += 1;
        }
    }
    println!("round-tripped {checked} JSON files ({bookmarks} bookmark files untyped)");
    if report_folders().len() == 3 {
        assert!(checked >= 100, "only {checked} files checked");
        assert_eq!(bookmarks, 3);
    }
}

/// `read_pbir` + `write_pbir` reproduces every file byte for byte.
#[test]
fn read_then_write_is_byte_identical() {
    let (mut total, mut identical) = (0, 0);
    for (label, folder) in report_folders() {
        let report = read_pbir(&DirSource(folder.clone())).unwrap();
        assert!(
            report.diagnostics.is_empty(),
            "{label}: {:?}",
            report.diagnostics
        );
        let written = write_pbir(&report);
        assert_eq!(
            written.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            report.files.keys().collect::<Vec<_>>(),
            "{label}: file set changed"
        );
        for (path, bytes) in written {
            let original = std::fs::read(folder.join(&path)).unwrap();
            total += 1;
            if bytes == original {
                identical += 1;
            } else {
                println!("{label}/{path}: bytes differ");
            }
            if is_json_path(&path) {
                let a: Value = serde_json::from_slice(&bytes).unwrap();
                let b: Value = serde_json::from_slice(&original).unwrap();
                assert_eq!(first_difference(&a, &b), None, "{label}/{path}");
            }
        }
    }
    println!("{identical}/{total} files byte-identical");
    assert_eq!(identical, total);
}

/// Edits survive: a moved visual is written with the new position, other
/// keys keep their order, and a deleted visual's file is dropped.
#[test]
fn edited_reports_write_minimal_changes() {
    let Some(folder) = common::report_folder("stores") else {
        return;
    };
    let mut report = read_pbir(&DirSource(folder)).unwrap();
    let page = &mut report.pages[0];
    let removed = page.visual_path(&page.visuals[1]);
    let moved = page.visual_path(&page.visuals[0]);
    page.visuals[0].visual.position.as_mut().unwrap().x = Some(serde_json::Number::from(5));
    page.visuals.remove(1);
    let written = write_pbir(&report);
    assert!(written.iter().all(|(p, _)| *p != removed));
    let (_, bytes) = written.iter().find(|(p, _)| *p == moved).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains("\"position\": {\n    \"x\": 5,\n    \"y\""),
        "{text}"
    );
}
