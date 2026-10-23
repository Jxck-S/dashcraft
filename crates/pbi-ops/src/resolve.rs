//! Name resolution shared by the commands: reports, pages, visuals, tables,
//! slicers; and report themes.

use pbi_core::ReportSource;
use pbi_model::{Model, Page, Report, Visual, dax_column_ref, names_eq};
use pbi_render::query::is_slicer_type;
use pbi_render::theme::ThemeResolver;
use pbi_session::Session;
use serde_json::json;

use crate::error::OpError;
use crate::workspace::DocEntry;

/// Fails unless the document has a local semantic model.
///
/// # Errors
/// `noDocument`.
pub fn require_model(entry: &DocEntry) -> Result<(), OpError> {
    if entry.session().has_model() {
        Ok(())
    } else {
        Err(OpError::no_document(format!(
            "{} ({}) has no local semantic model (a report bound to a remote model?)",
            entry.id(),
            entry.shown_path()
        )))
    }
}

/// Index of the report `name`: exact (case-insensitive) name, else the only
/// report whose name contains it; the first report when `None`.
///
/// # Errors
/// `noDocument` without reports, `notFound` / `invalidParams` (ambiguous).
pub fn report_index(session: &Session, name: Option<&str>) -> Result<usize, OpError> {
    let reports = &session.document().reports;
    if reports.is_empty() {
        return Err(OpError::no_document("the document has no reports"));
    }
    let names = || reports.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(0);
    };
    if let Some(i) = reports.iter().position(|r| names_eq(&r.name, name)) {
        return Ok(i);
    }
    if let Ok(n) = name.parse::<usize>()
        && (1..=reports.len()).contains(&n)
    {
        return Ok(n - 1);
    }
    let needle = name.to_lowercase();
    let mut partial = reports
        .iter()
        .enumerate()
        .filter(|(_, r)| r.name.to_lowercase().contains(&needle));
    match (partial.next(), partial.next()) {
        (Some((i, _)), None) => Ok(i),
        (Some(_), Some(_)) => Err(OpError::invalid_params(format!(
            "'{name}' matches several reports"
        ))
        .with_data(json!({ "reports": names() }))),
        _ => Err(OpError::not_found(format!("no report named '{name}'"))
            .with_data(json!({ "reports": names() }))),
    }
}

/// The report at `index`.
///
/// # Errors
/// `notFound`.
pub fn report_at(session: &Session, index: usize) -> Result<&Report, OpError> {
    session
        .document()
        .reports
        .get(index)
        .ok_or_else(|| OpError::not_found(format!("no report #{}", index + 1)))
}

/// Index of the page `spec` names in `report`: internal name, display name
/// (case-insensitive), 1-based index or a unique part of the display name;
/// the active (else first) page when `None`.
///
/// # Errors
/// `noDocument` for a report without pages, `notFound` / `invalidParams`.
pub fn page_index(report: &Report, spec: Option<&str>) -> Result<usize, OpError> {
    let pages = &report.pages;
    if pages.is_empty() {
        return Err(OpError::no_document(format!(
            "report '{}' has no pages",
            report.name
        )));
    }
    let list = || {
        pages
            .iter()
            .enumerate()
            .map(|(i, p)| json!({ "index": i + 1, "name": p.name, "displayName": p.display_name }))
            .collect::<Vec<_>>()
    };
    let Some(spec) = spec.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(report
            .active_page
            .as_deref()
            .and_then(|a| pages.iter().position(|p| p.name == a))
            .unwrap_or(0));
    };
    if let Some(i) = pages.iter().position(|p| p.name == spec) {
        return Ok(i);
    }
    if let Some(i) = pages.iter().position(|p| names_eq(&p.display_name, spec)) {
        return Ok(i);
    }
    if let Ok(n) = spec.parse::<usize>() {
        if (1..=pages.len()).contains(&n) {
            return Ok(n - 1);
        }
        return Err(OpError::not_found(format!(
            "page {n}: report '{}' has {} page(s)",
            report.name,
            pages.len()
        ))
        .with_data(json!({ "pages": list() })));
    }
    let needle = spec.to_lowercase();
    let mut partial = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.display_name.to_lowercase().contains(&needle));
    match (partial.next(), partial.next()) {
        (Some((i, _)), None) => Ok(i),
        (Some(_), Some(_)) => Err(OpError::invalid_params(format!(
            "page '{spec}' matches several pages"
        ))
        .with_data(json!({ "pages": list() }))),
        _ => Err(
            OpError::not_found(format!("no page '{spec}' in report '{}'", report.name))
                .with_data(json!({ "pages": list() })),
        ),
    }
}

/// The visual named `name` on `page`.
///
/// # Errors
/// `notFound`.
pub fn visual<'a>(page: &'a Page, name: &str) -> Result<&'a Visual, OpError> {
    page.visual(name).ok_or_else(|| {
        OpError::not_found(format!(
            "no visual '{name}' on page '{}'",
            page.display_name
        ))
        .with_data(json!({
            "visuals": page.visuals.iter().map(|v| json!({"name": v.name, "visualType": v.visual_type})).collect::<Vec<_>>()
        }))
    })
}

/// The model spelling of table `name` (case-insensitive).
///
/// # Errors
/// `notFound`, listing the tables.
pub fn table_name(model: &Model, name: &str) -> Result<String, OpError> {
    model.table(name).map(|t| t.name.clone()).ok_or_else(|| {
        OpError::not_found(format!("no table named '{name}'")).with_data(json!({
            "tables": model.tables.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
        }))
    })
}

/// The report's theme (the built-in default for PBIX layouts and
/// PBIR-Legacy reports).
#[must_use]
pub fn theme(session: &Session, report_index: usize) -> ThemeResolver {
    match session.opened().report_sources.get(report_index) {
        Some(ReportSource::Pbir { report, .. }) => ThemeResolver::from_pbir(report),
        _ => ThemeResolver::default(),
    }
}

/// Labels a slicer answers to: its fields' display names, column names and
/// `Table[Column]` / `Table.Column` references.
#[must_use]
pub fn slicer_labels(v: &Visual) -> Vec<String> {
    let mut labels = Vec::new();
    for p in v.projections_for("Values") {
        labels.push(p.label().to_owned());
        if let Some(c) = p.field.property() {
            labels.push(c.to_owned());
            if let Some(t) = p.field.entity() {
                labels.push(dax_column_ref(t, c));
                labels.push(format!("{t}.{c}"));
            }
        }
    }
    labels
}

/// The slicer on `page` named `visual` (exact visual name) or bound to
/// `field` (display name, column, `Table[Column]`, case-insensitive).
///
/// # Errors
/// `invalidParams` (neither or both given, ambiguous, not a slicer),
/// `notFound`.
pub fn slicer<'a>(
    page: &'a Page,
    visual: Option<&str>,
    field: Option<&str>,
) -> Result<&'a Visual, OpError> {
    let slicers = || {
        page.visuals
            .iter()
            .filter(|v| is_slicer_type(&v.visual_type))
            .map(|v| json!({ "visual": v.name, "fields": slicer_labels(v) }))
            .collect::<Vec<_>>()
    };
    match (visual, field) {
        (Some(name), None) => {
            let v = self::visual(page, name)?;
            if is_slicer_type(&v.visual_type) {
                Ok(v)
            } else {
                Err(OpError::invalid_params(format!(
                    "visual '{name}' is a {}, not a slicer",
                    v.visual_type
                )))
            }
        }
        (None, Some(field)) => {
            let matches: Vec<&Visual> = page
                .visuals
                .iter()
                .filter(|v| is_slicer_type(&v.visual_type))
                .filter(|v| slicer_labels(v).iter().any(|l| names_eq(l, field)))
                .collect();
            match matches.as_slice() {
                [v] => Ok(v),
                [] => Err(OpError::not_found(format!(
                    "no slicer on '{field}' on page '{}'",
                    page.display_name
                ))
                .with_data(json!({ "slicers": slicers() }))),
                several => Err(OpError::invalid_params(format!(
                    "'{field}' matches {} slicers on page '{}'; name the visual instead",
                    several.len(),
                    page.display_name
                ))
                .with_data(json!({ "slicers": slicers() }))),
            }
        }
        _ => Err(OpError::invalid_params(
            "name the slicer with exactly one of `visual` or `field`",
        )),
    }
}
