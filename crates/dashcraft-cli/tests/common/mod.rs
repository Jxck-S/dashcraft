//! Shared helpers of the `dashcraft` end-to-end tests: the synthetic Acme
//! Outfitters example (`reference/examples/acme-outfitters/`), its
//! `expected.json` and patched copies of it.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The example folder, relative to the fixture root.
pub const ACME: &str = "examples/acme-outfitters";
pub const PBIP: &str = "examples/acme-outfitters/AcmeOutfitters.pbip";
pub const BLOB_MAP: &str = "examples/acme-outfitters/blob-map.json";
pub const EXPECTED: &str = "examples/acme-outfitters/expected.json";
/// The Microsoft samples (MIT) and their facts.
pub const MS_PBIX: &str = "samples/microsoft/Revenue Opportunities.pbix";
pub const MS_EXPECTED: &str = "samples/microsoft/expected.json";

/// `$DASHCRAFT_REFERENCE`, else `<workspace>/reference`.
pub fn fixture_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The example's `expected.json` (computed by its generator, independently
/// of DashCraft), or `None` after printing `SKIP:`.
pub fn expected() -> Option<Expected> {
    let path = fixture_root().join(EXPECTED);
    if path.exists() {
        Some(Expected(read_json(&path)))
    } else {
        println!("SKIP: {} missing", path.display());
        None
    }
}

/// The `files[]` entries of `samples/microsoft/expected.json`, or `None`
/// after printing `SKIP:`.
pub fn ms_expected() -> Option<Vec<Value>> {
    let path = fixture_root().join(MS_EXPECTED);
    if !path.exists() {
        println!("SKIP: {} missing", path.display());
        return None;
    }
    Some(read_json(&path)["files"].as_array().unwrap().clone())
}

/// Accessors over the Acme `expected.json`.
pub struct Expected(pub Value);

impl Expected {
    pub fn reports(&self) -> &Vec<Value> {
        self.0["reports"].as_array().unwrap()
    }

    /// The report whose name ends with `short` (`Stores` → `Acme Outfitters - Stores`).
    pub fn report(&self, short: &str) -> &Value {
        self.reports()
            .iter()
            .find(|r| r["report"].as_str().unwrap().ends_with(short))
            .unwrap_or_else(|| panic!("no report {short} in expected.json"))
    }

    pub fn report_name(&self, short: &str) -> &str {
        self.report(short)["report"].as_str().unwrap()
    }

    /// `(name, value, formatted)` of every measure, unfiltered.
    pub fn measures(&self) -> Vec<(&str, &Value, &str)> {
        Self::entries(&self.0["measures"]["unfiltered"])
    }

    /// `(name, value, formatted)` of every measure under a report's filters.
    pub fn report_measures(&self, short: &str) -> Vec<(&str, &Value, &str)> {
        let name = self.report_name(short);
        Self::entries(&self.0["measures"]["byReport"][name]["values"])
    }

    fn entries(values: &Value) -> Vec<(&str, &Value, &str)> {
        values
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), &v["value"], v["formatted"].as_str().unwrap()))
            .collect()
    }

    pub fn measure(&self, report: Option<&str>, measure: &str) -> &str {
        let list = match report {
            Some(r) => self.report_measures(r),
            None => self.measures(),
        };
        list.into_iter()
            .find(|(n, ..)| *n == measure)
            .unwrap_or_else(|| panic!("no measure {measure} in expected.json"))
            .2
    }

    pub fn value(&self, measure: &str) -> &Value {
        &self.0["measures"]["unfiltered"][measure]["value"]
    }

    pub fn row_counts(&self) -> Vec<(&str, u64)> {
        self.0["rowCounts"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_u64().unwrap()))
            .collect()
    }

    pub fn rows(&self, table: &str) -> u64 {
        self.0["rowCounts"][table].as_u64().unwrap()
    }
}

/// Copies a folder recursively.
pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// The visual of the Executive Summary KPI card, relative to the example folder.
pub const KPI_CARD: &str = "Acme Outfitters - Overview.Report/definition/pages/ef796b39c816a4afa4f1/visuals/f7a9c9413fef68532313/visual.json";

/// Copies the example to `to` and gives the Executive Summary KPI card one
/// stale `queryRef` (`Orders.Sales Total` for `Orders.Total Sales`): exactly
/// one LINT003 warning.
pub fn acme_with_stale_query_ref(to: &Path) {
    copy_dir(&fixture_root().join(ACME), to);
    let visual = to.join(KPI_CARD);
    let text = fs::read_to_string(&visual).unwrap();
    let stale = text.replacen(
        "\"queryRef\": \"Orders.Total Sales\"",
        "\"queryRef\": \"Orders.Sales Total\"",
        1,
    );
    assert_ne!(stale, text, "the KPI card projects Orders.Total Sales");
    fs::write(&visual, stale).unwrap();
}
