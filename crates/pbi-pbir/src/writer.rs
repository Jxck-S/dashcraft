//! PBIR report folder writer.

use indexmap::IndexMap;
use serde::Serialize;

use crate::json::JsonStyle;
use crate::reader::{DEFINITION_PBIR, PAGES_JSON, PLATFORM, PbirReport, REPORT_JSON, VERSION_JSON};

/// Serializes a report folder back to `(path, bytes)` pairs.
///
/// - Typed files are serialized from their structs, keeping source key
///   order and number text, in the formatting detected from the original
///   bytes ([`JsonStyle::detect`]); new files use [`JsonStyle::default`]
///   (2-space indent, no trailing newline, as Power BI writes PBIR).
/// - Files that were not parsed (images, bookmarks, invalid JSON, …) are
///   written verbatim.
/// - Parsed files whose struct was removed (a deleted page or visual) are
///   omitted.
///
/// Output order: original files in path order, then new files.
pub fn write_pbir(report: &PbirReport) -> Vec<(String, Vec<u8>)> {
    let mut typed: IndexMap<String, Vec<u8>> = IndexMap::new();
    let mut put = |path: String, bytes: Option<Vec<u8>>| {
        if let Some(bytes) = bytes {
            typed.insert(path, bytes);
        }
    };
    if let Some(v) = &report.definition_pbir {
        put(DEFINITION_PBIR.into(), styled(report, DEFINITION_PBIR, v));
    }
    if let Some(v) = &report.platform {
        put(PLATFORM.into(), styled(report, PLATFORM, v));
    }
    if let Some(v) = &report.report {
        put(REPORT_JSON.into(), styled(report, REPORT_JSON, v));
    }
    if let Some(v) = &report.version {
        put(VERSION_JSON.into(), styled(report, VERSION_JSON, v));
    }
    if let Some(v) = &report.pages_meta {
        put(PAGES_JSON.into(), styled(report, PAGES_JSON, v));
    }
    for page in &report.pages {
        let path = page.path();
        let bytes = styled(report, &path, &page.page);
        put(path, bytes);
        for visual in &page.visuals {
            let path = page.visual_path(visual);
            let bytes = styled(report, &path, &visual.visual);
            put(path, bytes);
        }
    }
    for theme in &report.themes {
        put(
            theme.path.clone(),
            styled(report, &theme.path, &theme.theme),
        );
    }

    let mut out = Vec::with_capacity(report.files.len() + typed.len());
    for (path, file) in &report.files {
        if let Some(bytes) = typed.shift_remove(path) {
            out.push((path.clone(), bytes));
        } else if !file.parsed {
            out.push((path.clone(), file.bytes.clone()));
        }
    }
    out.extend(typed);
    out
}

/// Serializes `value` in the style of the original file at `path`. Falls
/// back to the original bytes if serialization fails.
fn styled<T: Serialize>(report: &PbirReport, path: &str, value: &T) -> Option<Vec<u8>> {
    let original = report.files.get(path);
    let style = original.map_or_else(JsonStyle::default, |f| JsonStyle::detect(&f.bytes));
    style
        .to_vec(value)
        .ok()
        .or_else(|| original.map(|f| f.bytes.clone()))
}
