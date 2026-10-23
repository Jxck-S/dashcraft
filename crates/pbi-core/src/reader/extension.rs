//! Extension packages that are identified and described, **never executed**:
//!
//! - `.pbiviz` custom visuals: a ZIP with `package.json` and
//!   `resources/<guid>.pbiviz.json` (`visual` {`name`, `displayName`,
//!   `guid`, `version`, `description`, `supportUrl`}, `apiVersion`, `author`,
//!   `capabilities` {`dataRoles`, `objects`, `privileges`}, `content` {`js`,
//!   `css`, `iconBase64`}). Only metadata is read; the script is measured,
//!   never evaluated, and the visual renders as a placeholder.
//! - `.mez` / `.pqx` Power Query custom connectors: an OPC ZIP with the
//!   connector's M section document (`*.m`/`*.pq`), resources and icons
//!   (`.pqx` adds a digital signature part). Entries are listed and the M
//!   sources are parsed (not evaluated) for the section name and the
//!   `shared` members with their `DataSource.Kind` / `Publish` attributes.

use std::sync::Arc;

use pbi_m::ast::{Expr, ExprKind, Literal};
use pbi_m::{Document as MDocument, parse_document};
use pbi_model::{Diagnostic, Origin, SourceFormat};
use serde_json::Value;

use super::Builder;
use super::codes;
use super::files::{read_json_capped, read_text_capped, root_file};
use super::outline::{
    Connector, ConnectorMember, ConnectorSource, CustomVisual, FormatOutline, VisualDataRole,
};

/// Largest `package.json` read.
const PACKAGE_MAX_BYTES: u64 = 1024 * 1024;
/// Largest `resources/*.pbiviz.json` read (it embeds the script).
const RESOURCE_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Largest connector M source parsed.
const SOURCE_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Most connector M sources parsed / entries listed / names kept.
const MAX_SOURCES: usize = 50;
const MAX_FILES: usize = 2_000;
const MAX_NAMES: usize = 500;

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The `resources/*.pbiviz.json` part: from `package.json` `resources[].file`,
/// else the first one in `resources/`.
fn resource_entry(b: &Builder<'_>, package: &Value) -> Option<String> {
    let declared = package
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| s(r.get("file")))
        .map(|f| f.replace('\\', "/"))
        .find(|f| f.ends_with(".pbiviz.json") && b.container.is_file(f));
    declared.or_else(|| {
        b.container
            .list_prefix("resources")
            .into_iter()
            .find(|e| !e.is_dir && e.path.ends_with(".pbiviz.json"))
            .map(|e| e.path.clone())
    })
}

pub(crate) fn read_pbiviz(b: &mut Builder<'_>) {
    b.doc.source.format = SourceFormat::Pbiviz;
    let container = Arc::clone(&b.container);
    let package = root_file(container.as_ref(), "package.json")
        .and_then(|p| read_json_capped(container.as_ref(), &p, PACKAGE_MAX_BYTES).ok())
        .map(|(v, _)| v)
        .unwrap_or(Value::Null);
    let resource_path = resource_entry(b, &package);
    let resource = match &resource_path {
        Some(path) => match read_json_capped(container.as_ref(), path, RESOURCE_MAX_BYTES) {
            Ok((v, _)) => v,
            Err(e) => {
                b.diag(
                    Diagnostic::warning(codes::EXTENSION, format!("{path}: {e}"))
                        .with_origin(Origin::file(path.clone())),
                );
                Value::Null
            }
        },
        None => Value::Null,
    };
    let visual = resource
        .get("visual")
        .or_else(|| package.get("visual"))
        .unwrap_or(&Value::Null);
    let author = resource
        .get("author")
        .or_else(|| package.get("author"))
        .unwrap_or(&Value::Null);
    let capabilities = resource.get("capabilities").unwrap_or(&Value::Null);
    let data_roles = capabilities
        .get("dataRoles")
        .and_then(Value::as_array)
        .map(|roles| {
            roles
                .iter()
                .take(MAX_NAMES)
                .filter_map(|r| {
                    Some(VisualDataRole {
                        name: s(r.get("name"))?,
                        display_name: s(r.get("displayName")),
                        kind: match r.get("kind") {
                            Some(Value::String(k)) => Some(k.clone()),
                            Some(Value::Number(n)) => Some(
                                match n.as_i64() {
                                    Some(0) => "Grouping",
                                    Some(1) => "Measure",
                                    Some(2) => "GroupingOrMeasure",
                                    _ => "unknown",
                                }
                                .to_owned(),
                            ),
                            _ => None,
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let objects = capabilities
        .get("objects")
        .and_then(Value::as_object)
        .map(|o| o.keys().take(MAX_NAMES).cloned().collect())
        .unwrap_or_default();
    let privileges: Vec<String> = capabilities
        .get("privileges")
        .and_then(Value::as_array)
        .map(|p| {
            p.iter()
                .take(MAX_NAMES)
                .filter_map(|x| s(x.get("name")))
                .collect()
        })
        .unwrap_or_default();
    let script_bytes = resource
        .get("content")
        .and_then(|c| c.get("js"))
        .and_then(Value::as_str)
        .map_or(0, |js| js.len() as u64);
    let info = CustomVisual {
        name: s(visual.get("name")),
        display_name: s(visual.get("displayName")),
        guid: s(visual.get("guid")),
        version: s(visual.get("version")).or_else(|| s(package.get("version"))),
        description: s(visual.get("description")),
        author: s(author.get("name")).or_else(|| s(Some(author))),
        api_version: s(resource.get("apiVersion")).or_else(|| s(package.get("apiVersion"))),
        support_url: s(visual.get("supportUrl")),
        data_roles,
        objects,
        privileges,
        script_bytes,
        resource: resource_path.clone(),
    };
    if info.guid.is_none() && info.name.is_none() {
        b.diag(Diagnostic::warning(
            codes::EXTENSION,
            "custom visual package without readable visual metadata (package.json / resources/*.pbiviz.json)",
        ));
    }
    if !info.privileges.is_empty() {
        b.diag(Diagnostic::info(
            codes::EXTENSION,
            format!(
                "the visual requests privileges: {} (never granted: the visual is not run)",
                info.privileges.join(", ")
            ),
        ));
    }
    b.diag(Diagnostic::info(
        codes::EXTENSION,
        format!(
            "custom visual {} {}: {} data role(s), {} bytes of script; metadata only, never executed (rendered as a placeholder)",
            info.display_name
                .as_deref()
                .or(info.name.as_deref())
                .unwrap_or("(unnamed)"),
            info.version.as_deref().unwrap_or("?"),
            info.data_roles.len(),
            info.script_bytes
        ),
    ));
    b.outline = Some(FormatOutline::CustomVisual(info));
}

/// A text literal attribute value.
fn literal_text(e: &Expr) -> Option<String> {
    match &e.unparenthesized().kind {
        ExprKind::Literal(Literal::Text(t)) => Some(t.clone()),
        _ => None,
    }
}

/// `DataSource.Kind` and `Publish` of a member attribute record.
fn attributes(record: Option<&Expr>) -> (Option<String>, Option<String>) {
    let Some(ExprKind::Record(fields)) = record.map(|r| &r.unparenthesized().kind) else {
        return (None, None);
    };
    let find = |name: &str| {
        fields
            .iter()
            .find(|f| f.name.text.eq_ignore_ascii_case(name))
            .and_then(|f| literal_text(&f.value))
    };
    (find("DataSource.Kind"), find("Publish"))
}

fn connector_source(b: &Builder<'_>, path: &str) -> ConnectorSource {
    let mut out = ConnectorSource {
        path: path.to_owned(),
        ..ConnectorSource::default()
    };
    let src = match read_text_capped(b.container.as_ref(), path, SOURCE_MAX_BYTES) {
        Ok((t, _)) => t,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    };
    match parse_document(src.trim_start_matches('\u{feff}')) {
        Ok(MDocument::Section(section)) => {
            out.section = Some(section.name.text.clone());
            out.members = section
                .members
                .iter()
                .take(MAX_NAMES)
                .map(|m| {
                    let (kind, publish) = attributes(m.attributes.as_ref());
                    ConnectorMember {
                        name: m.name.text.clone(),
                        shared: m.shared,
                        data_source_kind: kind,
                        publish,
                    }
                })
                .collect();
        }
        Ok(MDocument::Expression(_)) => {}
        Err(e) => out.error = Some(e.message),
    }
    out
}

pub(crate) fn read_connector(b: &mut Builder<'_>) {
    b.doc.source.format = SourceFormat::Connector;
    let files: Vec<String> = b
        .container
        .entries()
        .iter()
        .filter(|e| !e.is_dir)
        .take(MAX_FILES)
        .map(|e| e.path.clone())
        .collect();
    let signed = files.iter().any(|f| {
        let lower = f.to_ascii_lowercase();
        lower.contains("digital-signature")
            || lower.contains("_xmlsignatures")
            || [".psdsxs", ".psdor", ".p7s", ".sig"]
                .iter()
                .any(|ext| lower.ends_with(ext))
    });
    let sources: Vec<ConnectorSource> = files
        .iter()
        .filter(|f| {
            let lower = f.to_ascii_lowercase();
            [".m", ".pq", ".pqm"].iter().any(|ext| lower.ends_with(ext))
        })
        .take(MAX_SOURCES)
        .map(|f| connector_source(b, f))
        .collect();
    for src in sources.iter().filter(|s| s.error.is_some()) {
        b.diag(
            Diagnostic::warning(
                codes::M_SYNTAX,
                format!("{}: {}", src.path, src.error.as_deref().unwrap_or_default()),
            )
            .with_origin(Origin::file(src.path.clone())),
        );
    }
    let kinds: Vec<&str> = sources
        .iter()
        .flat_map(|s| s.members.iter())
        .filter_map(|m| m.data_source_kind.as_deref())
        .collect();
    b.diag(Diagnostic::info(
        codes::EXTENSION,
        format!(
            "custom connector: {} file(s){}{}; listed only, never executed",
            files.len(),
            if signed { ", signed" } else { "" },
            if kinds.is_empty() {
                String::new()
            } else {
                format!(", data source kind(s) {}", kinds.join(", "))
            }
        ),
    ));
    b.outline = Some(FormatOutline::Connector(Connector {
        files,
        signed,
        sources,
    }));
}
