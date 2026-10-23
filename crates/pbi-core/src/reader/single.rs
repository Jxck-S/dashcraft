//! Single-file inputs: TMSL `.bim`, a TMDL file, `.abf` backups, `.pbids`
//! connection files, legacy layout JSON, `.rdl` paginated reports, M scripts
//! (`.pq`, `.m`) and dataflow `model.json`.

use std::sync::Arc;

use pbi_model::{Diagnostic, Origin, SourceFormat};
use serde_json::Value;

use super::files::{BytesContainer, file_stem};
use super::model::{load_abf, load_tmdl, load_tmsl};
use super::report::load_legacy;
use super::{Builder, OpenedDocument, codes, source_format_of, unsupported};
use crate::container::Container;
use crate::detect::DetectedFormat;
use crate::error::CoreError;

pub(crate) fn read(mut b: Builder<'_>, file: &BytesContainer) -> Result<OpenedDocument, CoreError> {
    use DetectedFormat as F;
    let entry = file.entry_name().to_owned();
    let container: Arc<dyn Container> = Arc::clone(&b.container);
    let format = b.detection.format.clone();
    b.doc.source.format = source_format_of(&format);
    match &format {
        F::Bim => {
            if let Some(loaded) = load_tmsl(&container, "", &entry, &mut b.doc.diagnostics) {
                b.consumed.extend(loaded.consumed);
                b.doc.model = Some(loaded.model);
                b.model_source = Some(loaded.source);
            }
        }
        F::TmdlFile => {
            if let Some(loaded) = load_tmdl(&container, "", "", &mut b.doc.diagnostics) {
                b.consumed.extend(loaded.consumed);
                b.doc.model = Some(loaded.model);
                b.model_source = Some(loaded.source);
            }
        }
        F::Abf { .. } => {
            if let Some(loaded) = load_abf(
                &container,
                &entry,
                Some(file.data()),
                b.opts,
                &mut b.doc.diagnostics,
            ) {
                b.doc.model = Some(loaded.model);
                b.model_source = Some(loaded.source);
            }
        }
        F::Pbids => read_pbids(&mut b, &entry),
        F::Rdl => super::rdl::read(&mut b, &entry),
        F::PowerQueryScript => super::mashup::read_script(&mut b, &entry),
        F::DataflowModelJson => super::mashup::read_cdm(&mut b, &entry),
        F::CustomConnector | F::Pbiviz | F::Vpax | F::PowerQueryTemplate => {
            b.diag(Diagnostic::info(
                codes::NOT_IMPLEMENTED,
                format!(
                    "detected {} from the extension only, but the file is not a ZIP package: listed only, never executed",
                    format.label()
                ),
            ));
        }
        F::LegacyLayoutJson => {
            if let Some(mut loaded) =
                load_legacy(container.as_ref(), "", &entry, &mut b.doc.diagnostics)
            {
                if loaded.report.name.is_empty() {
                    loaded.report.name = file_stem(&b.name).to_owned();
                }
                b.consumed.extend(loaded.consumed);
                b.doc.reports.push(loaded.report);
                b.report_sources.push(loaded.source);
            }
        }
        F::Pbip | F::PbirJsonFile { .. } | F::FabricJsonFile { .. } => {
            b.diag(Diagnostic::info(
                codes::NOT_IMPLEMENTED,
                format!(
                    "{} is one file of a PBIP project; open its folder (or the `.pbip` file on disk) instead",
                    format.label()
                ),
            ));
        }
        F::Unknown => return Err(unsupported(&b.detection)),
        F::Pbix { .. }
        | F::Pbit
        | F::PbipSemanticModelFolder
        | F::PbipReportFolder
        | F::TmdlFolder
        | F::DataflowFolder
        | F::FabricItemFolder { .. } => {
            return Err(CoreError::Unsupported {
                format: format.label(),
                hint: format!(
                    "the file is not a ZIP archive or folder, so it cannot be read as one ({})",
                    b.detection.signals.join("; ")
                ),
            });
        }
        other => b.not_implemented(other),
    }
    Ok(b.finish())
}

/// `.pbids`: connection descriptors only; no model.
fn read_pbids(b: &mut Builder<'_>, entry: &str) {
    b.doc.source.format = SourceFormat::Pbids;
    let Some(i) = b.json_part(entry, "pbids") else {
        return;
    };
    let connections: Vec<Value> = b
        .parts
        .get(i)
        .and_then(|p| p.json.as_ref())
        .and_then(|j| j.get("connections"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if connections.is_empty() {
        b.diag(
            Diagnostic::warning(codes::PBIDS, "the .pbids file lists no connections")
                .with_origin(Origin::file(entry)),
        );
    }
    for (i, conn) in connections.iter().enumerate() {
        let details = conn.get("details");
        let protocol = details
            .and_then(|d| d.get("protocol"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let address = details
            .and_then(|d| d.get("address"))
            .map(describe_address)
            .unwrap_or_default();
        let mode = conn
            .get("mode")
            .and_then(Value::as_str)
            .map(|m| format!(", mode {m}"))
            .unwrap_or_default();
        b.diag(
            Diagnostic::info(
                codes::PBIDS,
                format!("connection {}: protocol `{protocol}`{address}{mode}", i + 1),
            )
            .with_origin(Origin::file(entry).with_pointer(format!("/connections/{i}"))),
        );
    }
    b.diag(Diagnostic::info(
        codes::NO_MODEL,
        "a .pbids file only describes data sources; there is no model or report",
    ));
}

fn describe_address(address: &Value) -> String {
    match address {
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| match v {
                    Value::String(s) => format!("{k}={s}"),
                    other => format!("{k}={other}"),
                })
                .collect();
            if parts.is_empty() {
                String::new()
            } else {
                format!(", address {}", parts.join(", "))
            }
        }
        Value::String(s) => format!(", address {s}"),
        _ => String::new(),
    }
}
