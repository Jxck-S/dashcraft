//! A report's query-reduction options (`slowDataSourceSettings` in PBIR
//! `report.json`, `config.slowDataSourceSettings` in a PBIX `Layout`).
//!
//! | Setting | Desktop option | DashCraft |
//! |---|---|---|
//! | `isCrossHighlightingDisabled` | Disable cross highlighting/filtering by default | Selected data points filter or highlight only the visuals whose interaction the page sets explicitly |
//! | `isSlicerSelectionsButtonEnabled` | Add an Apply button to each slicer | Slicer selections stay pending until applied (`canvas.applySlicers`, the app's Apply button) |
//! | `isFilterSelectionsButtonEnabled`, `isApplyAllButtonEnabled` | Apply buttons in the filter pane | Read and reported; the filter pane is read-only until filter editing lands |
//! | `isFieldWellButtonEnabled` | Apply button for field changes | Not relevant to reading view |
//!
//! An "Apply all slicers" button that is visible on a page also makes
//! slicer selections pending (`pbi_ops::canvas` decides, it knows the
//! buttons' actions).

use pbi_model::Report;
use serde::Serialize;
use serde_json::Value;

/// The query-reduction options of a report (all `false` by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReduction {
    /// Selected data points do not cross-filter or cross-highlight by
    /// default.
    pub cross_highlighting_disabled: bool,
    /// Each slicer has an Apply button.
    pub slicer_apply_button: bool,
    /// Each basic filter card has an Apply button.
    pub filter_apply_button: bool,
    /// The filter pane has one Apply button.
    pub filter_pane_apply_all: bool,
}

impl QueryReduction {
    /// Reads the options of `report`.
    pub fn of(report: &Report) -> Self {
        let settings = report
            .extra
            .get("slowDataSourceSettings")
            .or_else(|| report.extra.get("config.slowDataSourceSettings"))
            .map(pbi_model::PropValue::to_json);
        let flag = |key: &str| {
            settings
                .as_ref()
                .and_then(|s| s.get(key))
                .and_then(Value::as_bool)
                == Some(true)
        };
        Self {
            cross_highlighting_disabled: flag("isCrossHighlightingDisabled"),
            slicer_apply_button: flag("isSlicerSelectionsButtonEnabled"),
            filter_apply_button: flag("isFilterSelectionsButtonEnabled"),
            filter_pane_apply_all: flag("isApplyAllButtonEnabled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::PropValue;

    #[test]
    fn reads_pbir_and_legacy_settings() {
        let mut report = Report::default();
        assert_eq!(QueryReduction::of(&report), QueryReduction::default());
        report.extra.insert(
            "slowDataSourceSettings".into(),
            PropValue::Json(serde_json::json!({
                "isCrossHighlightingDisabled": true,
                "isSlicerSelectionsButtonEnabled": true,
                "isFilterSelectionsButtonEnabled": false,
            })),
        );
        let r = QueryReduction::of(&report);
        assert!(r.cross_highlighting_disabled && r.slicer_apply_button);
        assert!(!r.filter_apply_button && !r.filter_pane_apply_all);

        let mut legacy = Report::default();
        legacy.extra.insert(
            "config.slowDataSourceSettings".into(),
            PropValue::Json(serde_json::json!({ "isApplyAllButtonEnabled": true })),
        );
        assert!(QueryReduction::of(&legacy).filter_pane_apply_all);
    }
}
