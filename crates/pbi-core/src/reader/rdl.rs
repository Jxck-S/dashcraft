//! Paginated reports (`.rdl`, Report Builder / SSRS report definitions):
//! an outline of data sources, datasets, parameters and report items in a
//! [`PaginatedReport`]. There is no semantic model and no Power BI report;
//! rendering is not supported.
//!
//! Layout (RDL 2008–2016 namespaces `…/sqlserver/reporting/<yyyy/mm>/reportdefinition`):
//! `Report/DataSources/DataSource[@Name]` (`ConnectionProperties/DataProvider`,
//! `ConnectString`, `IntegratedSecurity`, or `DataSourceReference`),
//! `Report/DataSets/DataSet[@Name]` (`Query/DataSourceName`, `CommandType`,
//! `CommandText`, `QueryParameters`; `SharedDataSet`; `Fields/Field` with
//! `DataField` or `Value` and `rd:TypeName`; `Filters`),
//! `Report/ReportParameters/ReportParameter[@Name]`, and the items under
//! `Report/ReportSections/ReportSection/{Body,Page/PageHeader,Page/PageFooter}/ReportItems`
//! (RDL 2008: `Report/Body`, `Report/PageHeader`, `Report/PageFooter`).
//!
//! Security: the XML parser refuses DTDs (so no external entities and no
//! entity expansion: XXE and "billion laughs" inputs are rejected), caps the
//! node count, and the input size is capped. Embedded connection strings
//! are never stored: they become [`REDACTED`]. Embedded images are listed by
//! name only.

use pbi_model::{Diagnostic, Origin, SourceFormat};
use roxmltree::{Document, Node, ParsingOptions};

use super::Builder;
use super::codes;
use super::files::read_text_capped;
use super::outline::{
    FormatOutline, PaginatedReport, RdlDataSet, RdlDataSource, RdlField, RdlItem, RdlParameter,
    RdlQueryParameter, RdlSection,
};
use crate::redact::REDACTED;

/// Largest report definition read (embedded images can make them large).
const RDL_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// XML node cap.
const RDL_MAX_NODES: u32 = 4_000_000;
/// Report items listed (all sections, all levels).
const MAX_ITEMS: usize = 10_000;
/// Nesting depth of report items listed.
const MAX_ITEM_DEPTH: usize = 32;
/// Longest textbox value kept.
const MAX_VALUE_CHARS: usize = 200;

const ITEM_KINDS: &[(&str, &str)] = &[
    ("Tablix", "tablix"),
    ("Textbox", "textbox"),
    ("Chart", "chart"),
    ("Image", "image"),
    ("Rectangle", "rectangle"),
    ("Subreport", "subreport"),
    ("Line", "line"),
    ("Map", "map"),
    ("GaugePanel", "gaugePanel"),
    ("CustomReportItem", "customReportItem"),
];

fn is_elem(n: &Node<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| is_elem(n, name))
}

fn children<'a, 'i: 'a>(node: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |n| is_elem(n, name))
}

/// Child at a `/`-separated path of local names.
fn at<'a, 'i>(node: Node<'a, 'i>, path: &str) -> Option<Node<'a, 'i>> {
    path.split('/').try_fold(node, |n, name| child(n, name))
}

fn text_of(node: Node<'_, '_>) -> Option<String> {
    let value: String = node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn text(node: Node<'_, '_>, path: &str) -> Option<String> {
    at(node, path).and_then(text_of)
}

fn flag(node: Node<'_, '_>, path: &str) -> bool {
    text(node, path).is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

fn name_attr(node: Node<'_, '_>) -> String {
    node.attribute("Name").unwrap_or_default().to_owned()
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &s[..cut]),
        None => s.to_owned(),
    }
}

/// `…/reporting/2016/01/reportdefinition` → `2016/01`.
fn schema_of(namespace: &str) -> Option<String> {
    let rest = namespace.split("/reporting/").nth(1)?;
    let version = rest.strip_suffix("/reportdefinition")?;
    (!version.is_empty()).then(|| version.to_owned())
}

struct Items {
    count: usize,
    truncated: bool,
}

impl Items {
    fn kind(node: &Node<'_, '_>) -> Option<&'static str> {
        if !node.is_element() {
            return None;
        }
        let name = node.tag_name().name();
        ITEM_KINDS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
    }

    /// The nearest report items below `node` (inside `ReportItems` or
    /// tablix `CellContents`), without recursion over the XML tree.
    fn below(&mut self, node: Node<'_, '_>, depth: usize) -> Vec<RdlItem> {
        let mut out = Vec::new();
        let mut stack: Vec<Node<'_, '_>> = node.children().filter(Node::is_element).collect();
        stack.reverse();
        while let Some(n) = stack.pop() {
            let in_container = n
                .parent_element()
                .is_some_and(|p| is_elem(&p, "ReportItems") || is_elem(&p, "CellContents"));
            if let (Some(kind), true) = (Self::kind(&n), in_container) {
                if self.count >= MAX_ITEMS || depth >= MAX_ITEM_DEPTH {
                    self.truncated = true;
                    continue;
                }
                out.push(self.item(n, kind, depth));
                continue;
            }
            let start = stack.len();
            stack.extend(n.children().filter(Node::is_element));
            stack[start..].reverse();
        }
        out
    }

    fn item(&mut self, node: Node<'_, '_>, kind: &str, depth: usize) -> RdlItem {
        self.count += 1;
        let value = (kind == "textbox").then(|| textbox_value(node)).flatten();
        RdlItem {
            kind: kind.to_owned(),
            name: name_attr(node),
            dataset: text(node, "DataSetName"),
            value,
            report_name: text(node, "ReportName"),
            children: self.below(node, depth + 1),
        }
    }

    fn list(&mut self, container: Option<Node<'_, '_>>) -> Vec<RdlItem> {
        match container.and_then(|c| child(c, "ReportItems")) {
            Some(items) => self.below(items, 0),
            None => Vec::new(),
        }
    }
}

fn textbox_value(node: Node<'_, '_>) -> Option<String> {
    let runs: Vec<String> = node
        .descendants()
        .filter(|n| is_elem(n, "TextRun"))
        .filter_map(|r| text(r, "Value"))
        .take(20)
        .collect();
    (!runs.is_empty()).then(|| truncate(&runs.join(" "), MAX_VALUE_CHARS))
}

fn data_source(node: Node<'_, '_>) -> RdlDataSource {
    let props = child(node, "ConnectionProperties");
    RdlDataSource {
        name: name_attr(node),
        data_provider: props.and_then(|p| text(p, "DataProvider")),
        connection_string: props
            .and_then(|p| child(p, "ConnectString"))
            .map(|_| REDACTED.to_owned()),
        shared_reference: text(node, "DataSourceReference"),
        integrated_security: props
            .and_then(|p| text(p, "IntegratedSecurity"))
            .map(|v| v.eq_ignore_ascii_case("true")),
    }
}

fn data_set(node: Node<'_, '_>) -> RdlDataSet {
    let query = child(node, "Query");
    RdlDataSet {
        name: name_attr(node),
        data_source: query.and_then(|q| text(q, "DataSourceName")),
        command_type: query.and_then(|q| text(q, "CommandType")),
        command_text: query.and_then(|q| text(q, "CommandText")),
        shared_reference: text(node, "SharedDataSet/SharedDataSetReference"),
        fields: child(node, "Fields")
            .map(|f| {
                children(f, "Field")
                    .map(|field| RdlField {
                        name: name_attr(field),
                        data_field: text(field, "DataField"),
                        value: text(field, "Value"),
                        type_name: text(field, "TypeName"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        query_parameters: query
            .and_then(|q| child(q, "QueryParameters"))
            .map(|qp| {
                children(qp, "QueryParameter")
                    .map(|p| RdlQueryParameter {
                        name: name_attr(p),
                        value: text(p, "Value"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        filters: child(node, "Filters").map_or(0, |f| children(f, "Filter").count()),
    }
}

fn parameter(node: Node<'_, '_>) -> RdlParameter {
    let default = child(node, "DefaultValue");
    let valid = child(node, "ValidValues");
    RdlParameter {
        name: name_attr(node),
        data_type: text(node, "DataType"),
        prompt: text(node, "Prompt"),
        nullable: flag(node, "Nullable"),
        allow_blank: flag(node, "AllowBlank"),
        multi_value: flag(node, "MultiValue"),
        hidden: flag(node, "Hidden"),
        default_values: default
            .and_then(|d| child(d, "Values"))
            .map(|v| children(v, "Value").filter_map(text_of).take(100).collect())
            .unwrap_or_default(),
        default_dataset: default.and_then(|d| text(d, "DataSetReference/DataSetName")),
        valid_values_dataset: valid.and_then(|v| text(v, "DataSetReference/DataSetName")),
        valid_values: valid
            .and_then(|v| child(v, "ParameterValues"))
            .map_or(0, |p| children(p, "ParameterValue").count()),
    }
}

/// Parses a report definition into its outline; the flag is `true` if a data
/// source is a Power BI / Analysis Services semantic model (decided from the
/// raw connection string, which is not kept).
pub(crate) fn parse_rdl(text_doc: &str) -> Result<(PaginatedReport, bool), String> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: RDL_MAX_NODES,
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(text_doc.trim_start_matches('\u{feff}'), options)
        .map_err(|e| format!("XML refused or malformed: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Report" {
        return Err(format!(
            "root element is <{}>, not <Report>",
            root.tag_name().name()
        ));
    }
    let mut report = PaginatedReport {
        schema: root.tag_name().namespace().and_then(schema_of),
        description: text(root, "Description"),
        author: text(root, "Author"),
        ..PaginatedReport::default()
    };
    let mut semantic_model = false;
    if let Some(sources) = child(root, "DataSources") {
        for node in children(sources, "DataSource") {
            let ds = data_source(node);
            let raw = text(node, "ConnectionProperties/ConnectString");
            semantic_model |= is_semantic_model(&ds, raw.as_deref());
            report.data_sources.push(ds);
        }
    }
    if let Some(sets) = child(root, "DataSets") {
        report.datasets = children(sets, "DataSet").map(data_set).collect();
    }
    if let Some(params) = child(root, "ReportParameters") {
        report.parameters = children(params, "ReportParameter").map(parameter).collect();
    }
    if let Some(images) = child(root, "EmbeddedImages") {
        report.embedded_images = children(images, "EmbeddedImage").map(name_attr).collect();
    }
    let mut items = Items {
        count: 0,
        truncated: false,
    };
    match child(root, "ReportSections") {
        Some(sections) => {
            for section in children(sections, "ReportSection") {
                let page = child(section, "Page");
                report.sections.push(RdlSection {
                    page_width: page.and_then(|p| text(p, "PageWidth")),
                    page_height: page.and_then(|p| text(p, "PageHeight")),
                    body: items.list(child(section, "Body")),
                    header: items.list(page.and_then(|p| child(p, "PageHeader"))),
                    footer: items.list(page.and_then(|p| child(p, "PageFooter"))),
                });
            }
        }
        None => {
            let page = child(root, "Page");
            report.sections.push(RdlSection {
                page_width: page
                    .and_then(|p| text(p, "PageWidth"))
                    .or_else(|| text(root, "PageWidth")),
                page_height: page
                    .and_then(|p| text(p, "PageHeight"))
                    .or_else(|| text(root, "PageHeight")),
                body: items.list(child(root, "Body")),
                header: items.list(
                    child(root, "PageHeader").or_else(|| page.and_then(|p| child(p, "PageHeader"))),
                ),
                footer: items.list(
                    child(root, "PageFooter").or_else(|| page.and_then(|p| child(p, "PageFooter"))),
                ),
            });
        }
    }
    report.item_count = items.count;
    report.truncated = items.truncated;
    Ok((report, semantic_model))
}

/// `true` if a data source points at a Power BI / Analysis Services model.
fn is_semantic_model(ds: &RdlDataSource, raw: Option<&str>) -> bool {
    let provider = ds.data_provider.as_deref().unwrap_or_default();
    provider.eq_ignore_ascii_case("PBIDATASET")
        || raw.is_some_and(|c| {
            let c = c.to_ascii_lowercase();
            c.contains("powerbi://") || c.contains("pbiazure://")
        })
}

pub(crate) fn read(b: &mut Builder<'_>, entry: &str) {
    b.doc.source.format = SourceFormat::Rdl;
    let text_doc = match read_text_capped(b.container.as_ref(), entry, RDL_MAX_BYTES) {
        Ok((t, _)) => t,
        Err(e) => {
            b.diag(
                Diagnostic::error(codes::PAGINATED, format!("{entry}: {e}"))
                    .with_origin(Origin::file(entry)),
            );
            return;
        }
    };
    let (report, remote) = match parse_rdl(&text_doc) {
        Ok(r) => r,
        Err(e) => {
            b.diag(
                Diagnostic::error(codes::PAGINATED, format!("{entry}: {e}"))
                    .with_origin(Origin::file(entry)),
            );
            return;
        }
    };
    if remote {
        b.remote = true;
        b.diag(
            Diagnostic::info(
                codes::REMOTE_MODEL,
                format!(
                    "a dataset queries a Power BI / Analysis Services semantic model: {}",
                    super::REMOTE_DESCRIPTION
                ),
            )
            .with_origin(Origin::file(entry)),
        );
    }
    let redacted = report
        .data_sources
        .iter()
        .filter(|d| d.connection_string.is_some())
        .count();
    if redacted > 0 {
        b.diag(
            Diagnostic::info(
                codes::REDACTED,
                format!("{redacted} embedded connection string(s) redacted from {entry}"),
            )
            .with_origin(Origin::file(entry)),
        );
    }
    if report.truncated {
        b.diag(
            Diagnostic::warning(
                codes::PAGINATED,
                format!(
                    "report items listed up to {MAX_ITEMS} items / {MAX_ITEM_DEPTH} levels; the rest were left out"
                ),
            )
            .with_origin(Origin::file(entry)),
        );
    }
    b.diag(
        Diagnostic::info(
            codes::PAGINATED,
            format!(
                "paginated report: {} data source(s), {} dataset(s), {} parameter(s), {} report item(s); outline only, not rendered",
                report.data_sources.len(),
                report.datasets.len(),
                report.parameters.len(),
                report.item_count
            ),
        )
        .with_origin(Origin::file(entry)),
    );
    b.outline = Some(FormatOutline::PaginatedReport(report));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_comes_from_the_namespace() {
        assert_eq!(
            schema_of("http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition")
                .as_deref(),
            Some("2016/01")
        );
        assert_eq!(schema_of("urn:x"), None);
        assert_eq!(truncate("abcdef", 3), "abc…");
    }

    #[test]
    fn utf16_declarations_are_accepted_after_decoding() {
        let xml = r#"<?xml version="1.0" encoding="utf-16"?><Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2010/01/reportdefinition"><Body><ReportItems><Textbox Name="T"/></ReportItems></Body></Report>"#;
        let (report, remote) = parse_rdl(xml).unwrap();
        assert!(!remote);
        assert_eq!(report.schema.as_deref(), Some("2010/01"));
        assert_eq!(report.sections[0].body[0].name, "T");
    }
}
