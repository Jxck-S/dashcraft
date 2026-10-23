//! Excel workbooks (`.xlsx`, `.xlsm`, `.xlsb`). A workbook with a Power
//! Pivot data model stores it as an Analysis Services backup in
//! `xl/model/item.data`: an uncompressed ABF (sometimes XPress9 like a PBIX
//! `DataModel`) whose files are individually Xpress-compressed
//! (`ApplyCompression`) and followed by CRC markers. Its metadata is legacy
//! XML (compatibility level 1100/1103), read by `pbi-vertipaq` together with
//! the VertiPaq storage map (`.tbl.xml`), so the session can decode the data
//! like a PBIX (`ModelSource::Abf`). Sheets, connections and everything else
//! in the workbook are listed, not read.

use std::sync::Arc;

use pbi_model::{Diagnostic, Origin, PropValue, SourceFormat};

use super::model::load_abf;
use super::{Builder, ModelSource, codes};
use crate::detect::DetectedFormat;

/// The Power Pivot backup entry.
pub(crate) const ITEM_DATA: &str = "xl/model/item.data";

pub(crate) fn read(b: &mut Builder<'_>) {
    b.doc.source.format = SourceFormat::PowerPivot;
    let container = Arc::clone(&b.container);
    if b.detection.format == DetectedFormat::ExcelNoModel || !container.is_file(ITEM_DATA) {
        b.diag(Diagnostic::info(
            codes::NO_MODEL,
            "Excel workbook without a Power Pivot data model (no xl/model/item.data); sheets are not read",
        ));
        return;
    }
    if !b.opts.load_pbix_schema {
        b.diag(
            Diagnostic::info(
                codes::NO_MODEL,
                "Power Pivot data model present; its schema was not loaded (load_pbix_schema is off)",
            )
            .with_origin(Origin::file(ITEM_DATA)),
        );
        return;
    }
    let Some(loaded) = load_abf(&container, ITEM_DATA, None, b.opts, &mut b.doc.diagnostics) else {
        return;
    };
    let xml = loaded
        .model
        .extra
        .get("vertipaq.metadataFormat")
        .and_then(PropValue::as_str)
        == Some("xml");
    let per_file =
        matches!(&loaded.source, ModelSource::Abf { archive, .. } if archive.is_compressed());
    b.diag(
        Diagnostic::info(
            codes::POWER_PIVOT,
            format!(
                "Power Pivot data model: {} table(s), {} metadata{}",
                loaded.model.tables.len(),
                if xml {
                    "legacy XML (compatibility level 1100/1103)"
                } else {
                    "tabular (metadata.sqlitedb)"
                },
                if per_file {
                    "; files Xpress-compressed inside the backup"
                } else {
                    ""
                }
            ),
        )
        .with_origin(Origin::file(ITEM_DATA)),
    );
    b.consumed.insert(ITEM_DATA.to_owned());
    b.doc.model = Some(loaded.model);
    b.model_source = Some(loaded.source);
}
