//! Report loading: PBIR folders, PBIR-Legacy `report.json` and the PBIX
//! `Report/Layout`, plus origin rebasing and `byConnection` sanitizing.

use pbi_model::{DatasetReference, Diagnostic, Filter, Origin, Report};
use pbi_pbir::{DefinitionPbir, read_legacy_layout_at, read_pbir};
use serde_json::Value;

use super::files::{ContainerFileSource, join};
use super::{ReportSource, codes};
use crate::container::Container;
use crate::redact::sanitize_connection_string;

/// A loaded report and its source.
pub(crate) struct LoadedReport {
    pub report: Report,
    pub source: ReportSource,
    /// Entries interpreted as the report definition.
    pub consumed: Vec<String>,
}

/// Reads the PBIR report rooted at `folder` (`definition/` inside it).
pub(crate) fn load_pbir(
    c: &dyn Container,
    folder: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<LoadedReport> {
    let sub = c.sub(folder);
    let src = ContainerFileSource::new(&sub);
    let result = read_pbir(&src);
    for (path, e) in src.take_errors() {
        let path = join(folder, &path);
        diags.push(
            Diagnostic::warning(codes::READ_FAILED, format!("{path}: {e}"))
                .with_origin(Origin::file(path)),
        );
    }
    let pbir = match result {
        Ok(p) => p,
        Err(e) => {
            diags.push(
                Diagnostic::error(
                    codes::REPORT_FAILED,
                    format!("report `{}` not loaded: {e}", display(folder)),
                )
                .with_origin(Origin::file(folder)),
            );
            return None;
        }
    };
    let (mut report, conversion) = pbi_pbir::to_report_ast(&pbir);
    let mut report_diags: Vec<Diagnostic> = pbir.diagnostics.clone();
    report_diags.extend(conversion);
    rebase_diags(&mut report_diags, folder);
    diags.extend(report_diags);
    rebase_report(&mut report, folder);
    sanitize_dataset_reference(&mut report.dataset_reference);
    let consumed = pbir
        .files
        .iter()
        .filter(|(path, file)| file.parsed && path.starts_with("definition/"))
        .map(|(path, _)| join(folder, path))
        .collect();
    Some(LoadedReport {
        report,
        source: ReportSource::Pbir {
            folder: folder.to_owned(),
            report: Box::new(pbir),
        },
        consumed,
    })
}

/// Reads a legacy layout entry (`Report/Layout`, `<folder>/report.json`).
pub(crate) fn load_legacy(
    c: &dyn Container,
    folder: &str,
    entry: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<LoadedReport> {
    let fail = |diags: &mut Vec<Diagnostic>, msg: String| {
        diags.push(Diagnostic::error(codes::REPORT_FAILED, msg).with_origin(Origin::file(entry)));
        None
    };
    let bytes = match c.read(entry) {
        Ok(b) => b,
        Err(e) => return fail(diags, format!("{entry}: {e}")),
    };
    let legacy = match read_legacy_layout_at(&bytes, entry) {
        Ok(l) => l,
        Err(e) => return fail(diags, format!("report layout {entry} not loaded: {e}")),
    };
    let (mut report, conversion) = pbi_pbir::legacy_to_report_ast(&legacy);
    diags.extend(legacy.diagnostics.iter().cloned());
    diags.extend(conversion);
    if let Some(reference) = definition_dataset_reference(c, folder) {
        report.dataset_reference = reference;
    }
    if report.name.is_empty()
        && let Some(name) = platform_display_name(c, folder)
    {
        report.name = name;
    }
    Some(LoadedReport {
        report,
        source: ReportSource::Legacy {
            folder: folder.to_owned(),
            entry: entry.to_owned(),
            report: Box::new(legacy),
        },
        consumed: vec![entry.to_owned()],
    })
}

fn display(folder: &str) -> &str {
    if folder.is_empty() { "." } else { folder }
}

fn read_small_json(c: &dyn Container, path: &str) -> Option<Value> {
    super::files::read_json(c, path).ok().map(|(v, _)| v)
}

fn platform_display_name(c: &dyn Container, folder: &str) -> Option<String> {
    read_small_json(c, &join(folder, ".platform"))?
        .get("metadata")?
        .get("displayName")?
        .as_str()
        .map(str::to_owned)
}

/// The `datasetReference` of `<folder>/definition.pbir` (PBIR-Legacy
/// folders; PBIR folders get it from `pbi-pbir`).
fn definition_dataset_reference(c: &dyn Container, folder: &str) -> Option<DatasetReference> {
    let json = read_small_json(c, &join(folder, "definition.pbir"))?;
    let def: DefinitionPbir = serde_json::from_value(json).ok()?;
    if let Some(path) = def.by_path() {
        return Some(DatasetReference::ByPath {
            path: path.to_owned(),
        });
    }
    let conn = def.by_connection()?;
    let mut reference = DatasetReference::ByConnection {
        connection_string: conn.connection_string.clone(),
        model_id: conn.pbi_service_model_id.as_ref().and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }),
        database_name: conn.pbi_model_database_name.clone(),
        connection_type: conn.connection_type.clone(),
        name: conn.name.clone(),
        raw: serde_json::to_value(conn).unwrap_or(Value::Null),
    };
    sanitize_dataset_reference(&mut reference);
    Some(reference)
}

/// Masks secrets in a `byConnection` connection string (see
/// [`crate::redact`]); the server and model stay readable.
pub(crate) fn sanitize_dataset_reference(reference: &mut DatasetReference) {
    if let DatasetReference::ByConnection {
        connection_string,
        raw,
        ..
    } = reference
    {
        if let Some(s) = connection_string {
            *s = sanitize_connection_string(s);
        }
        if let Some(Value::String(s)) = raw.get_mut("connectionString") {
            *s = sanitize_connection_string(s);
        }
    }
}

fn rebase_origin(origin: &mut Option<Origin>, prefix: &str) {
    if let Some(o) = origin {
        o.path = join(prefix, &o.path);
    }
}

fn rebase_filters(filters: &mut [Filter], prefix: &str) {
    for f in filters {
        rebase_origin(&mut f.origin, prefix);
    }
}

/// Makes report-relative origins container-relative (`definition/…` →
/// `<prefix>/definition/…`).
pub(crate) fn rebase_report(report: &mut Report, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    rebase_origin(&mut report.origin, prefix);
    rebase_filters(&mut report.filters, prefix);
    for page in &mut report.pages {
        rebase_origin(&mut page.origin, prefix);
        rebase_filters(&mut page.filters, prefix);
        for visual in &mut page.visuals {
            rebase_origin(&mut visual.origin, prefix);
            rebase_filters(&mut visual.filters, prefix);
        }
    }
}

/// As [`rebase_report`] for diagnostics.
pub(crate) fn rebase_diags(diags: &mut [Diagnostic], prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    for d in diags {
        rebase_origin(&mut d.origin, prefix);
    }
}
