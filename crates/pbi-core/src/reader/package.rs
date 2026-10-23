//! PBIX and PBIT packages (ZIP archives, or unzipped folders).

use std::sync::Arc;

use pbi_model::{DatasetReference, Diagnostic, Origin, SourceFormat};
use serde_json::Value;

use super::files::{file_stem, item_name};
use super::model::{load_abf, load_tmsl};
use super::report::{load_legacy, load_pbir};
use super::{Builder, codes};
use crate::detect::{DetectedFormat, PbixReport};

/// JSON parts of a package, parsed into [`super::MetadataPart`]s.
const JSON_PARTS: &[(&str, &str)] = &[
    ("Settings", "settings"),
    ("Metadata", "metadata"),
    ("DiagramLayout", "diagramLayout"),
    ("Connections", "connections"),
];

pub(crate) fn read(b: &mut Builder<'_>, pbit: bool) {
    b.doc.source.format = if pbit {
        SourceFormat::Pbit
    } else {
        SourceFormat::Pbix
    };
    b.text_part("Version", "version");
    let mut connections = None;
    for (name, kind) in JSON_PARTS {
        if let Some(i) = b.json_part(name, kind)
            && *name == "Connections"
        {
            connections = b.parts.get(i).and_then(|p| p.json.clone());
        }
    }
    let container = Arc::clone(&b.container);
    if container.is_file("SecurityBindings") {
        b.diag(
            Diagnostic::info(
                codes::OPAQUE,
                "SecurityBindings is opaque (DPAPI-protected) and is never decoded",
            )
            .with_origin(Origin::file("SecurityBindings")),
        );
    }
    if container.is_file("DataMashup") {
        b.diag(
            Diagnostic::info(
                codes::OPAQUE,
                "DataMashup (binary Power Query package) is listed but not decoded yet",
            )
            .with_origin(Origin::file("DataMashup")),
        );
    }

    // Model.
    let live = live_connection(connections.as_ref());
    let has_data_model = container.is_file("DataModel");
    if pbit || (!has_data_model && container.is_file("DataModelSchema")) {
        if let Some(loaded) = load_tmsl(&container, "", "DataModelSchema", &mut b.doc.diagnostics) {
            b.consumed.extend(loaded.consumed);
            b.doc.model = Some(loaded.model);
            b.model_source = Some(loaded.source);
        }
    } else if has_data_model {
        if b.opts.load_pbix_schema {
            if let Some(loaded) = load_abf(
                &container,
                "DataModel",
                None,
                b.opts,
                &mut b.doc.diagnostics,
            ) {
                b.doc.model = Some(loaded.model);
                b.model_source = Some(loaded.source);
            }
        } else {
            b.diag(
                Diagnostic::info(
                    codes::NO_MODEL,
                    "DataModel present; its schema was not loaded (load_pbix_schema is off)",
                )
                .with_origin(Origin::file("DataModel")),
            );
        }
    } else if live.is_some() {
        b.remote = true;
        b.diag(
            Diagnostic::info(
                codes::REMOTE_MODEL,
                format!(
                    "live-connected report (no DataModel; model in the service): {}",
                    super::REMOTE_DESCRIPTION
                ),
            )
            .with_origin(Origin::file("Connections")),
        );
    } else {
        b.diag(Diagnostic::info(
            codes::NO_MODEL,
            "no DataModel or DataModelSchema entry and no live connection: the model is not available",
        ));
    }

    // Report.
    let report_flavour = match &b.detection.format {
        DetectedFormat::Pbix { report, .. } => *report,
        _ => {
            if container.is_dir("Report/definition") {
                PbixReport::Pbir
            } else if container.is_file("Report/Layout") {
                PbixReport::LegacyLayout
            } else {
                PbixReport::None
            }
        }
    };
    let loaded = match report_flavour {
        PbixReport::Pbir => load_pbir(container.as_ref(), "Report", &mut b.doc.diagnostics),
        PbixReport::LegacyLayout => load_legacy(
            container.as_ref(),
            "Report",
            "Report/Layout",
            &mut b.doc.diagnostics,
        ),
        PbixReport::None => {
            b.diag(Diagnostic::info(
                codes::NO_REPORT,
                "no `Report/definition/` or `Report/Layout`",
            ));
            None
        }
    };
    if let Some(mut loaded) = loaded {
        if loaded.report.name.is_empty() {
            loaded.report.name = item_name(file_stem(&b.name));
        }
        loaded.report.dataset_reference = match &live {
            Some(conn) if !has_data_model && !pbit => conn.clone(),
            _ => DatasetReference::Embedded,
        };
        b.consumed.extend(loaded.consumed);
        b.doc.reports.push(loaded.report);
        b.report_sources.push(loaded.source);
    }
}

/// The first live connection of a (redacted) `Connections` part.
fn live_connection(connections: Option<&Value>) -> Option<DatasetReference> {
    let conn = connections?
        .get("Connections")?
        .as_array()?
        .iter()
        .find(|c| c.is_object())?;
    let s = |key: &str| conn.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(DatasetReference::ByConnection {
        connection_string: None,
        model_id: conn.get("PbiServiceModelId").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }),
        database_name: s("PbiModelDatabaseName"),
        connection_type: s("ConnectionType"),
        name: s("Name"),
        raw: conn.clone(),
    })
}
