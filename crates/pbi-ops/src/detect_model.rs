//! Stable JSON schema of `pbiview detect --json` and the `doc.detect`
//! command (`schemaVersion: 1`), shared by the CLI and the registry.

use pbi_core::detect::{DatasetReference, Detection, ModelFlavour, ReportFlavour};
use pbi_core::reader::source_format_of;
use schemars::JsonSchema;
use serde::Serialize;

use crate::inspect_model::{DetectionOut, SCHEMA_VERSION, detection_out};

/// `detect --json` output.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DetectOut {
    pub schema_version: u32,
    /// The path as given.
    pub path: String,
    /// The `source.format` `inspect` reports for this input.
    pub source_format: String,
    pub detection: DetectionOut,
    pub items: ItemsOut,
}

/// Structure found while detecting (paths relative to the input folder,
/// the `.pbip` folder or the archive root).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemsOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pbip_file: Option<String>,
    pub semantic_models: Vec<ModelItemOut>,
    pub reports: Vec<ReportItemOut>,
    pub other_items: Vec<OtherItemOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelItemOut {
    pub path: String,
    /// `tmdl`, `bim` or `unknown`.
    pub flavour: String,
    pub has_cache_abf: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportItemOut {
    pub path: String,
    /// `pbir`, `pbirLegacy`, `legacyLayout` or `unknown`.
    pub flavour: String,
    /// `byPath`, `byConnection` or `unknown`.
    pub dataset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OtherItemOut {
    pub path: String,
    pub item_type: String,
}

/// The `detect --json` output of `d` (`path` as given).
pub fn detect_out(path: &str, d: &Detection) -> DetectOut {
    let parts = &d.parts;
    DetectOut {
        schema_version: SCHEMA_VERSION,
        path: path.to_owned(),
        source_format: source_format_of(&d.format).as_str().to_owned(),
        detection: detection_out(d),
        items: ItemsOut {
            pbip_file: parts.pbip_file.clone(),
            semantic_models: parts
                .semantic_models
                .iter()
                .map(|m| ModelItemOut {
                    path: m.path.clone(),
                    flavour: match m.flavour {
                        ModelFlavour::Tmdl => "tmdl",
                        ModelFlavour::Bim => "bim",
                        ModelFlavour::Unknown => "unknown",
                    }
                    .to_owned(),
                    has_cache_abf: m.has_cache_abf,
                    display_name: m.platform.as_ref().and_then(|p| p.display_name.clone()),
                })
                .collect(),
            reports: parts
                .reports
                .iter()
                .map(|r| {
                    let (dataset, dataset_path) = match &r.dataset {
                        DatasetReference::ByPath { path, .. } => ("byPath", Some(path.clone())),
                        DatasetReference::ByConnection => ("byConnection", None),
                        DatasetReference::Unknown => ("unknown", None),
                    };
                    ReportItemOut {
                        path: r.path.clone(),
                        flavour: match r.flavour {
                            ReportFlavour::Pbir => "pbir",
                            ReportFlavour::PbirLegacy => "pbirLegacy",
                            ReportFlavour::LegacyLayout => "legacyLayout",
                            ReportFlavour::Unknown => "unknown",
                        }
                        .to_owned(),
                        dataset: dataset.to_owned(),
                        dataset_path,
                        display_name: r.platform.as_ref().and_then(|p| p.display_name.clone()),
                    }
                })
                .collect(),
            other_items: parts
                .other_items
                .iter()
                .map(|i| OtherItemOut {
                    path: i.path.clone(),
                    item_type: i.item_type.clone(),
                })
                .collect(),
        },
    }
}
