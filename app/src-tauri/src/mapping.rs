//! pbi-model / pbi-core → IPC DTO mapping ([`crate::dto`]).
//!
//! Everything here is pure and infallible: unknown enum values become their raw spelling,
//! missing data becomes `None`, and recursive structures (filter conditions, aggregations) are
//! formatted with depth and length caps, so hostile files can neither blow the stack nor bloat
//! the IPC payload.

use pbi_core::OpenedDocument;
use pbi_core::detect::{self, Confidence, Detection};
use pbi_model::{
    Annotation, Cardinality, ColumnKind, Condition, CrossFilter, DatasetReference, Diagnostic,
    ExpressionKind, FieldExpr, Filter, Literal, LiteralValue, Model, ModelStatus, ObjectId,
    ObjectKind, Origin, PartitionSource, Projection, PropertyBag, Report, Severity, TableKind,
    Visual, dax_column_ref, dax_measure_ref, dax_table_ref,
};
use serde_json::Value;

use crate::dto::*;

/// Longest filter summary / title sent to the UI (characters).
const MAX_SUMMARY_CHARS: usize = 400;
/// Longest visual title (characters).
const MAX_TITLE_CHARS: usize = 120;
/// Nesting depth after which conditions and fields are elided.
const MAX_DEPTH: usize = 16;
/// Values listed in an `IN {…}` summary before `… (+N more)`.
const MAX_IN_VALUES: usize = 12;

/// `ObjectId` → IPC handle. IDs are dense, document-unique counters starting at 1, so they fit
/// a `u32` for any real document; the saturating fallback only exists to stay panic-free.
pub(crate) fn id(id: ObjectId) -> ObjectIdDto {
    u32::try_from(id.get()).unwrap_or(u32::MAX)
}

pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn origin(origin: Option<&Origin>) -> Option<String> {
    origin.map(ToString::to_string)
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// NaN and ±∞ (which JSON would carry as `null`) become 0.
fn finite(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

pub(crate) fn model_status(status: &ModelStatus) -> ModelStatusDto {
    match status {
        ModelStatus::Local => ModelStatusDto {
            availability: ModelAvailability::Local,
            description: None,
        },
        ModelStatus::Remote { description } => ModelStatusDto {
            availability: ModelAvailability::Remote,
            description: non_empty(Some(description))
                .or_else(|| Some(pbi_core::REMOTE_DESCRIPTION.to_owned())),
        },
        ModelStatus::Missing => ModelStatusDto {
            availability: ModelAvailability::Missing,
            description: Some("no semantic model found".to_owned()),
        },
    }
}

pub(crate) fn counts(opened: &OpenedDocument) -> DocumentCounts {
    let doc = &opened.document;
    let mut c = DocumentCounts {
        reports: count(doc.reports.len()),
        ..DocumentCounts::default()
    };
    if let Some(model) = &doc.model {
        c.tables = count(model.tables.len());
        c.columns = count(model.tables.iter().map(|t| t.columns.len()).sum());
        c.measures = count(model.tables.iter().map(|t| t.measures.len()).sum());
        c.relationships = count(model.relationships.len());
        c.expressions = count(model.expressions.len());
        c.parameters = count(model.parameters().count());
    }
    c.pages = count(doc.reports.iter().map(|r| r.pages.len()).sum());
    c.visuals = count(
        doc.reports
            .iter()
            .flat_map(|r| r.visuals())
            .filter(|(_, v)| !v.is_group())
            .count(),
    );
    c
}

pub(crate) fn diagnostic_counts(diagnostics: &[Diagnostic]) -> DiagnosticCounts {
    let by = |s: Severity| count(diagnostics.iter().filter(|d| d.severity == s).count());
    DiagnosticCounts {
        errors: by(Severity::Error),
        warnings: by(Severity::Warning),
        infos: by(Severity::Info),
    }
}

/// Filesystem facts about the opened path, gathered before parsing.
#[derive(Debug, Clone)]
pub(crate) struct PathInfo {
    pub path: String,
    pub file_name: String,
    pub entry_kind: EntryKind,
    pub size_bytes: Option<u64>,
}

pub(crate) fn summary(opened: &OpenedDocument, doc_id: &str, info: &PathInfo) -> DocumentSummary {
    let doc = &opened.document;
    DocumentSummary {
        doc_id: doc_id.to_owned(),
        path: info.path.clone(),
        file_name: info.file_name.clone(),
        entry_kind: info.entry_kind,
        size_bytes: info.size_bytes,
        format: doc.source.format.as_str().to_owned(),
        format_label: opened.detection.format.label(),
        model_status: model_status(&doc.model_status),
        model_name: doc.model.as_ref().map(|m| m.name.clone()),
        counts: counts(opened),
        diagnostics: diagnostic_counts(&doc.diagnostics),
        cache: None,
    }
}

// ---------------------------------------------------------------------------
// Model tree
// ---------------------------------------------------------------------------

pub(crate) fn model_tree(opened: &OpenedDocument) -> ModelTreeDto {
    let doc = &opened.document;
    let status = model_status(&doc.model_status);
    let source = opened.model_source.as_ref();
    let Some(model) = &doc.model else {
        return ModelTreeDto {
            id: None,
            name: None,
            status,
            description: None,
            culture: None,
            compatibility_level: None,
            source_kind: None,
            source_folder: None,
            tables: Vec::new(),
            relationships: Vec::new(),
            expressions: Vec::new(),
        };
    };
    ModelTreeDto {
        id: Some(id(model.id)),
        name: Some(model.name.clone()),
        status,
        description: non_empty(model.description.as_deref()),
        culture: model.culture.clone(),
        compatibility_level: model
            .compatibility_level()
            .and_then(|l| u32::try_from(l).ok()),
        source_kind: source.map(|s| s.kind().to_owned()),
        source_folder: source.map(|s| s.folder().to_owned()),
        tables: model.tables.iter().map(table).collect(),
        relationships: model.relationships.iter().map(relationship).collect(),
        expressions: model.expressions.iter().map(expression).collect(),
    }
}

pub(crate) fn table_kind(kind: TableKind) -> TableKindDto {
    match kind {
        TableKind::Regular => TableKindDto::Regular,
        TableKind::CalculatedTable => TableKindDto::CalculatedTable,
        TableKind::CalculationGroup => TableKindDto::CalculationGroup,
        TableKind::Remote => TableKindDto::Remote,
    }
}

fn table(t: &pbi_model::Table) -> TableDto {
    TableDto {
        id: id(t.id),
        name: t.name.clone(),
        description: non_empty(t.description.as_deref()),
        hidden: t.is_hidden,
        kind: table_kind(t.kind()),
        data_category: t.data_category.clone(),
        expression: t.calculated_expression().map(str::to_owned),
        columns: t.columns.iter().map(column).collect(),
        measures: t.measures.iter().map(measure).collect(),
        partitions: t.partitions.iter().map(partition).collect(),
        hierarchies: t
            .hierarchies
            .iter()
            .map(|h| HierarchyDto {
                id: id(h.id),
                name: h.name.clone(),
                hidden: h.is_hidden,
                levels: h.levels.iter().map(|l| l.name.clone()).collect(),
            })
            .collect(),
    }
}

fn column_kind(kind: &ColumnKind) -> ColumnKindDto {
    match kind {
        ColumnKind::Data { .. } => ColumnKindDto::Data,
        ColumnKind::Calculated { .. } => ColumnKindDto::Calculated,
        ColumnKind::CalculatedTableColumn { .. } => ColumnKindDto::CalculatedTableColumn,
        ColumnKind::RowNumber => ColumnKindDto::RowNumber,
    }
}

fn column(c: &pbi_model::Column) -> ColumnDto {
    ColumnDto {
        id: id(c.id),
        name: c.name.clone(),
        data_type: c.data_type.as_str().to_owned(),
        kind: column_kind(&c.kind),
        hidden: c.is_hidden,
        is_key: c.is_key,
        description: non_empty(c.description.as_deref()),
        source_column: non_empty(c.source_column()),
        expression: c.expression().map(str::to_owned),
        format_string: c.format_string.clone(),
        display_folder: c.display_folder.clone(),
        sort_by_column: c.sort_by_column.clone(),
        summarize_by: c.summarize_by.as_ref().map(|s| s.as_str().to_owned()),
        data_category: c.data_category.clone(),
    }
}

fn measure(m: &pbi_model::Measure) -> MeasureDto {
    MeasureDto {
        id: id(m.id),
        name: m.name.clone(),
        expression: m.expression.clone(),
        format_string: m.format_string.clone(),
        display_folder: m.display_folder.clone(),
        description: non_empty(m.description.as_deref()),
        hidden: m.is_hidden,
    }
}

fn partition_kind(source: &PartitionSource) -> PartitionKind {
    match source {
        PartitionSource::M { .. } => PartitionKind::M,
        PartitionSource::Calculated { .. } => PartitionKind::Calculated,
        PartitionSource::Entity { .. } => PartitionKind::Entity,
        PartitionSource::Query { .. } => PartitionKind::Query,
        PartitionSource::Unknown { .. } => PartitionKind::Other,
    }
}

fn partition(p: &pbi_model::Partition) -> PartitionDto {
    PartitionDto {
        id: id(p.id),
        name: p.name.clone(),
        kind: partition_kind(&p.source),
        source_type: p.source.kind_name().to_owned(),
        mode: p.mode.as_str().to_owned(),
        expression: p.source.expression().map(str::to_owned),
        query_group: p.query_group.clone(),
    }
}

fn cardinality(r: &pbi_model::Relationship) -> RelationshipCardinality {
    use Cardinality::{Many, One};
    match (&r.from_cardinality, &r.to_cardinality) {
        (Many, One) => RelationshipCardinality::ManyToOne,
        (One, Many) => RelationshipCardinality::OneToMany,
        (One, One) => RelationshipCardinality::OneToOne,
        (Many, Many) => RelationshipCardinality::ManyToMany,
        _ => RelationshipCardinality::Unknown,
    }
}

fn cross_filter(c: &CrossFilter) -> CrossFilterDirection {
    match c {
        CrossFilter::OneDirection => CrossFilterDirection::Single,
        CrossFilter::BothDirections => CrossFilterDirection::Both,
        CrossFilter::Automatic => CrossFilterDirection::Automatic,
        CrossFilter::Unknown(_) => CrossFilterDirection::Unknown,
    }
}

fn relationship(r: &pbi_model::Relationship) -> RelationshipDto {
    RelationshipDto {
        id: id(r.id),
        name: r.name.clone(),
        from_table: r.from.table.clone(),
        from_column: r.from.column.clone(),
        to_table: r.to.table.clone(),
        to_column: r.to.column.clone(),
        cardinality: cardinality(r),
        cross_filter: cross_filter(&r.cross_filtering_behavior),
        active: r.is_active,
    }
}

fn expression_language(kind: &ExpressionKind) -> CodeLanguage {
    match kind {
        ExpressionKind::M => CodeLanguage::M,
        ExpressionKind::Dax => CodeLanguage::Dax,
        ExpressionKind::Unknown(_) => CodeLanguage::Text,
    }
}

fn expression(e: &pbi_model::NamedExpression) -> ExpressionDto {
    ExpressionDto {
        id: id(e.id),
        name: e.name.clone(),
        language: expression_language(&e.kind),
        expression: e.full_expression(),
        is_parameter: e.is_parameter(),
        parameter_type: e.parameter_type(),
        query_group: e.query_group.clone(),
        description: non_empty(e.description.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Report outline
// ---------------------------------------------------------------------------

pub(crate) fn report_outline(opened: &OpenedDocument) -> Vec<ReportDto> {
    opened
        .document
        .reports
        .iter()
        .enumerate()
        .map(|(i, r)| ReportDto {
            theme: crate::views::report_theme(opened, i),
            ..report(r)
        })
        .collect()
}

fn dataset(reference: &DatasetReference) -> DatasetReferenceDto {
    match reference {
        DatasetReference::ByPath { path } => DatasetReferenceDto {
            kind: DatasetKind::ByPath,
            detail: non_empty(Some(path)),
        },
        DatasetReference::ByConnection {
            model_id,
            database_name,
            name,
            ..
        } => DatasetReferenceDto {
            kind: DatasetKind::ByConnection,
            detail: name
                .as_deref()
                .or(database_name.as_deref())
                .or(model_id.as_deref())
                .and_then(|s| non_empty(Some(s))),
        },
        DatasetReference::Embedded => DatasetReferenceDto {
            kind: DatasetKind::Embedded,
            detail: None,
        },
        DatasetReference::None => DatasetReferenceDto {
            kind: DatasetKind::None,
            detail: None,
        },
    }
}

fn report(r: &Report) -> ReportDto {
    ReportDto {
        id: id(r.id),
        name: r.name.clone(),
        dataset: dataset(&r.dataset_reference),
        start_page: r.start_page().map(|p| p.name.clone()),
        filters: r
            .filters
            .iter()
            .map(|f| filter(f, FilterScope::Report))
            .collect(),
        pages: r.pages.iter().map(page).collect(),
        origin: origin(r.origin.as_ref()),
        theme: ReportThemeDto {
            base_theme: r.theme.base_theme.as_ref().map(|t| t.name.clone()),
            custom_theme: r.theme.custom_theme.as_ref().map(|t| t.name.clone()),
            ..ReportThemeDto::default()
        },
    }
}

fn page(p: &pbi_model::Page) -> PageDto {
    PageDto {
        id: id(p.id),
        name: p.name.clone(),
        display_name: if p.display_name.trim().is_empty() {
            p.name.clone()
        } else {
            p.display_name.clone()
        },
        width: finite(p.width),
        height: finite(p.height),
        hidden: p.is_hidden,
        display_option: p.display_option.as_str().to_owned(),
        filters: p
            .filters
            .iter()
            .map(|f| filter(f, FilterScope::Page))
            .collect(),
        visuals: p.visuals.iter().map(visual).collect(),
    }
}

fn visual(v: &Visual) -> VisualDto {
    VisualDto {
        id: id(v.id),
        name: v.name.clone(),
        visual_type: if v.is_group() {
            "group".to_owned()
        } else {
            v.visual_type.clone()
        },
        is_group: v.is_group(),
        title: visual_title(v),
        position: PositionDto {
            x: finite(v.position.x),
            y: finite(v.position.y),
            z: finite(v.position.z),
            width: finite(v.position.width),
            height: finite(v.position.height),
        },
        hidden: v.is_hidden,
        parent_group: v.parent_group_name.clone(),
        fields: v.projections.iter().map(field).collect(),
        filters: v
            .filters
            .iter()
            .map(|f| filter(f, FilterScope::Visual))
            .collect(),
    }
}

/// Best-effort label: group name, `visualContainerObjects.title` text, or textbox text.
pub(crate) fn visual_title(v: &Visual) -> Option<String> {
    if let Some(name) = non_empty(v.display_name.as_deref()) {
        return Some(truncate(&name, MAX_TITLE_CHARS));
    }
    for objects in [&v.container_objects, &v.objects] {
        let raw = objects
            .pointer("/title/0/properties/text/expr/Literal/Value")
            .and_then(Value::as_str);
        if let Some(Ok(LiteralValue::Text(text))) = raw.map(|r| Literal::new(r).value())
            && !text.trim().is_empty()
        {
            return Some(truncate(text.trim(), MAX_TITLE_CHARS));
        }
    }
    let paragraphs = v
        .objects
        .pointer("/general/0/properties/paragraphs")
        .and_then(Value::as_array)?;
    let mut text = String::new();
    for run in paragraphs
        .iter()
        .filter_map(|p| p.get("textRuns").and_then(Value::as_array))
        .flatten()
    {
        if let Some(value) = run.get("value").and_then(Value::as_str) {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(value.trim());
        }
        if text.chars().count() > MAX_TITLE_CHARS {
            break;
        }
    }
    non_empty(Some(&text)).map(|t| truncate(&t, MAX_TITLE_CHARS))
}

fn field_kind(e: &FieldExpr) -> FieldKindDto {
    match e {
        FieldExpr::Column { .. } => FieldKindDto::Column,
        FieldExpr::Measure { .. } => FieldKindDto::Measure,
        FieldExpr::Aggregation { .. } => FieldKindDto::Aggregation,
        FieldExpr::HierarchyLevel { .. } => FieldKindDto::HierarchyLevel,
        FieldExpr::Literal(_) => FieldKindDto::Literal,
        FieldExpr::Unknown(_) => FieldKindDto::Unknown,
    }
}

pub(crate) fn field(p: &Projection) -> FieldDto {
    let display_name = p.label().to_owned();
    let field_name = p.field.property().map(str::to_owned);
    let aggregation = match &p.field {
        FieldExpr::Aggregation { function, .. } => Some(
            function
                .name()
                .map_or_else(|| format!("code {}", function.code()), str::to_owned),
        ),
        _ => None,
    };
    FieldDto {
        role: p.role.clone(),
        kind: field_kind(&p.field),
        renamed: p.display_name.is_some() && field_name.as_deref() != Some(display_name.as_str()),
        display_name,
        source_name: format_field(&p.field),
        table: p.field.entity().map(str::to_owned),
        field: field_name,
        aggregation,
        query_ref: p.query_ref.clone(),
    }
}

/// `Stores[Channel]`, `'Web Sessions'[Device]`, `Sum(Orders[Quantity])`.
pub(crate) fn format_field(e: &FieldExpr) -> String {
    format_field_at(e, 0)
}

fn format_field_at(e: &FieldExpr, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "…".to_owned();
    }
    match e {
        FieldExpr::Column { entity, property } | FieldExpr::Measure { entity, property } => {
            dax_column_ref(entity, property)
        }
        FieldExpr::Aggregation { expr, function } => {
            let name = function
                .query_ref_name()
                .map_or_else(|| format!("Aggregate{}", function.code()), str::to_owned);
            format!("{name}({})", format_field_at(expr, depth + 1))
        }
        FieldExpr::HierarchyLevel {
            entity,
            hierarchy,
            level,
            ..
        } => format!(
            "{}{}.{}",
            dax_table_ref(entity),
            dax_measure_ref(hierarchy),
            dax_measure_ref(level)
        ),
        FieldExpr::Literal(l) => format_literal(l),
        FieldExpr::Unknown(_) => "(expression)".to_owned(),
    }
}

/// DAX-style literal: `"Retail"`, `42`, `TRUE`, `BLANK()`.
fn format_literal(l: &Literal) -> String {
    match l.value() {
        Ok(LiteralValue::Text(s)) => format!("\"{}\"", s.replace('"', "\"\"")),
        Ok(LiteralValue::Integer(n)) => n.to_string(),
        Ok(LiteralValue::Double(f) | LiteralValue::Decimal(f)) => f.to_string(),
        Ok(LiteralValue::Boolean(b)) => if b { "TRUE" } else { "FALSE" }.to_owned(),
        Ok(LiteralValue::DateTime(s)) => format!("dt\"{s}\""),
        Ok(LiteralValue::Null) => "BLANK()".to_owned(),
        Err(_) => l.raw.clone(),
    }
}

fn filter(f: &Filter, scope: FilterScope) -> FilterDto {
    FilterDto {
        id: id(f.id),
        name: f.name.clone(),
        scope,
        display_name: non_empty(f.display_name.as_deref()),
        field: f.field.as_ref().map(format_field),
        filter_type: f.kind.as_str().to_owned(),
        summary: filter_summary(f),
        hidden: f.is_hidden_in_view_mode,
        locked: f.is_locked_in_view_mode,
    }
}

/// DAX-like condition text, e.g. `Stores[Channel] IN {"Retail"}`.
pub(crate) fn filter_summary(f: &Filter) -> String {
    match &f.condition {
        Some(c) => truncate(&format_condition(c, 0), MAX_SUMMARY_CHARS),
        None => "All values (no condition)".to_owned(),
    }
}

fn comparison_operator(kind: pbi_model::ComparisonKind) -> &'static str {
    use pbi_model::ComparisonKind as K;
    match kind {
        K::Equal => "=",
        K::GreaterThan => ">",
        K::GreaterThanOrEqual => ">=",
        K::LessThan => "<",
        K::LessThanOrEqual => "<=",
    }
}

fn format_in(exprs: &[FieldExpr], values: &[Vec<Literal>], negated: bool, depth: usize) -> String {
    let fields: Vec<String> = exprs
        .iter()
        .map(|e| format_field_at(e, depth + 1))
        .collect();
    let lhs = match fields.as_slice() {
        [single] => single.clone(),
        many => format!("({})", many.join(", ")),
    };
    let mut items: Vec<String> = values
        .iter()
        .take(MAX_IN_VALUES)
        .map(|row| {
            let cells: Vec<String> = row.iter().map(format_literal).collect();
            match cells.as_slice() {
                [single] => single.clone(),
                many => format!("({})", many.join(", ")),
            }
        })
        .collect();
    if values.len() > MAX_IN_VALUES {
        items.push(format!("… (+{} more)", values.len() - MAX_IN_VALUES));
    }
    let op = if negated { "NOT IN" } else { "IN" };
    format!("{lhs} {op} {{{}}}", items.join(", "))
}

/// Operands of a chain of the same boolean operator, flattened iteratively so long
/// `a AND b AND c …` chains (left-deep trees) do not recurse once per operand.
fn flatten(c: &Condition, and: bool) -> Vec<&Condition> {
    let mut out = Vec::new();
    let mut stack = vec![c];
    while let Some(c) = stack.pop() {
        match (c, and) {
            (Condition::And(a, b), true) | (Condition::Or(a, b), false) => {
                stack.push(b);
                stack.push(a);
            }
            _ => out.push(c),
        }
        if out.len() > MAX_SUMMARY_CHARS {
            break;
        }
    }
    out
}

fn format_condition(c: &Condition, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "…".to_owned();
    }
    let field = |e: &FieldExpr| format_field_at(e, depth + 1);
    match c {
        Condition::In { exprs, values } => format_in(exprs, values, false, depth),
        Condition::Not(inner) => match inner.as_ref() {
            Condition::In { exprs, values } => format_in(exprs, values, true, depth),
            other => format!("NOT ({})", format_condition(other, depth + 1)),
        },
        Condition::Comparison { kind, left, right } => {
            format!(
                "{} {} {}",
                field(left),
                comparison_operator(*kind),
                field(right)
            )
        }
        Condition::And(..) | Condition::Or(..) => {
            let and = matches!(c, Condition::And(..));
            let parts: Vec<String> = flatten(c, and)
                .into_iter()
                .map(|p| {
                    let text = format_condition(p, depth + 1);
                    if matches!(p, Condition::And(..) | Condition::Or(..)) {
                        format!("({text})")
                    } else {
                        text
                    }
                })
                .collect();
            parts.join(if and { " AND " } else { " OR " })
        }
        Condition::Contains { left, right } => {
            format!("CONTAINSSTRING({}, {})", field(left), field(right))
        }
        Condition::StartsWith { left, right } => {
            format!("{} STARTS WITH {}", field(left), field(right))
        }
        Condition::Between { expr, lower, upper } => format!(
            "{} BETWEEN {} AND {}",
            field(expr),
            field(lower),
            field(upper)
        ),
        Condition::Unknown(_) => "(unsupported condition)".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Diagnostics, detection
// ---------------------------------------------------------------------------

fn severity(s: Severity) -> SeverityDto {
    match s {
        Severity::Error => SeverityDto::Error,
        Severity::Warning => SeverityDto::Warning,
        Severity::Info => SeverityDto::Info,
    }
}

pub(crate) fn diagnostic(d: &Diagnostic) -> DiagnosticDto {
    DiagnosticDto {
        severity: severity(d.severity),
        code: d.code.clone(),
        message: d.message.clone(),
        origin: origin(d.origin.as_ref()),
        object_id: d.object.filter(|o| o.is_assigned()).map(id),
    }
}

pub(crate) fn diagnostics(opened: &OpenedDocument) -> Vec<DiagnosticDto> {
    opened.document.diagnostics.iter().map(diagnostic).collect()
}

fn confidence(c: Confidence) -> ConfidenceDto {
    match c {
        Confidence::NoSignal => ConfidenceDto::NoSignal,
        Confidence::ExtensionOnly => ConfidenceDto::ExtensionOnly,
        Confidence::Likely => ConfidenceDto::Likely,
        Confidence::Certain => ConfidenceDto::Certain,
    }
}

/// The serde spelling of a unit enum (or the `kind` tag of a tagged one), e.g. `pbirLegacy`.
fn serde_name<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        Ok(Value::Object(map)) => map
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn dataset_detail(d: &detect::DatasetReference) -> Option<String> {
    match d {
        detect::DatasetReference::ByPath { path, .. } => Some(format!("byPath {path}")),
        detect::DatasetReference::ByConnection => {
            Some(format!("byConnection ({})", pbi_core::REMOTE_DESCRIPTION))
        }
        detect::DatasetReference::Unknown => None,
    }
}

pub(crate) fn detection(opened: &OpenedDocument) -> DetectionDto {
    let det: &Detection = &opened.detection;
    let semantic_models = det
        .parts
        .semantic_models
        .iter()
        .map(|m| DetectedItemDto {
            path: m.path.clone(),
            flavour: serde_name(&m.flavour),
            detail: m.has_cache_abf.then(|| ".pbi/cache.abf present".to_owned()),
        })
        .collect();
    let reports = det
        .parts
        .reports
        .iter()
        .map(|r| DetectedItemDto {
            path: r.path.clone(),
            flavour: serde_name(&r.flavour),
            detail: dataset_detail(&r.dataset),
        })
        .collect();
    let mut parts: Vec<PartDto> = opened
        .parts
        .iter()
        .map(|p| PartDto {
            path: p.path.clone(),
            kind: p.kind.clone(),
            size_bytes: None,
        })
        .collect();
    for p in &opened.document.extra_parts {
        match parts.iter_mut().find(|known| known.path == p.path) {
            Some(known) => known.size_bytes = known.size_bytes.or(p.size),
            None => parts.push(PartDto {
                path: p.path.clone(),
                kind: p.kind.clone(),
                size_bytes: p.size,
            }),
        }
    }
    DetectionDto {
        format: serde_name(&det.format),
        label: det.format.label(),
        confidence: confidence(det.confidence),
        signals: det.signals.clone(),
        pbip_file: det.parts.pbip_file.clone(),
        semantic_models,
        reports,
        entry_count: count(opened.document.source.entries.len()),
        parts,
    }
}

// ---------------------------------------------------------------------------
// Object details
// ---------------------------------------------------------------------------

/// Longest code block / JSON text sent to the UI (bytes, cut at a char boundary).
const MAX_CODE_BYTES: usize = 256 * 1024;
/// Longest value of an `extra` property (characters).
const MAX_EXTRA_CHARS: usize = 2_000;

#[derive(Default)]
struct Sheet {
    properties: Vec<PropertyDto>,
    code: Vec<CodeBlockDto>,
}

impl Sheet {
    fn push(&mut self, label: &str, value: impl Into<String>, mono: bool, tooltip: Option<String>) {
        self.properties.push(PropertyDto {
            label: label.to_owned(),
            value: value.into(),
            tooltip,
            mono,
        });
    }

    fn text(&mut self, label: &str, value: impl Into<String>) {
        self.push(label, value, false, None);
    }

    fn mono(&mut self, label: &str, value: impl Into<String>) {
        self.push(label, value, true, None);
    }

    fn opt(&mut self, label: &str, value: Option<&str>) {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            self.text(label, v);
        }
    }

    fn opt_mono(&mut self, label: &str, value: Option<&str>) {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            self.mono(label, v);
        }
    }

    fn flag(&mut self, label: &str, value: bool) {
        self.text(label, if value { "Yes" } else { "No" });
    }

    fn code(&mut self, label: impl Into<String>, language: CodeLanguage, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.code.push(CodeBlockDto {
            label: label.into(),
            language,
            text: cap_bytes(text),
        });
    }

    fn json(&mut self, label: &str, value: &Value) {
        if value.is_null() || value.as_object().is_some_and(serde_json::Map::is_empty) {
            return;
        }
        if let Ok(text) = serde_json::to_string_pretty(value) {
            self.code(label, CodeLanguage::Json, &text);
        }
    }
}

fn cap_bytes(text: &str) -> String {
    if text.len() <= MAX_CODE_BYTES {
        return text.to_owned();
    }
    let mut cut = MAX_CODE_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n… (truncated, {} bytes total)",
        &text[..cut],
        text.len()
    )
}

fn annotations(list: &[Annotation]) -> Vec<PropertyDto> {
    list.iter()
        .map(|a| PropertyDto {
            label: a.name.clone(),
            value: truncate(&a.value, MAX_EXTRA_CHARS),
            tooltip: None,
            mono: true,
        })
        .collect()
}

fn extra(bag: &PropertyBag) -> Vec<PropertyDto> {
    bag.iter()
        .map(|(k, v)| {
            let json = v.to_json();
            let value = match &json {
                Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            PropertyDto {
                label: k.clone(),
                value: truncate(&value, MAX_EXTRA_CHARS),
                tooltip: None,
                mono: true,
            }
        })
        .collect()
}

fn object_kind(kind: ObjectKind) -> ObjectKindDto {
    match kind {
        ObjectKind::Model => ObjectKindDto::Model,
        ObjectKind::Database => ObjectKindDto::Database,
        ObjectKind::Table => ObjectKindDto::Table,
        ObjectKind::Column => ObjectKindDto::Column,
        ObjectKind::Measure => ObjectKindDto::Measure,
        ObjectKind::Partition => ObjectKindDto::Partition,
        ObjectKind::Hierarchy => ObjectKindDto::Hierarchy,
        ObjectKind::Level => ObjectKindDto::Level,
        ObjectKind::Relationship => ObjectKindDto::Relationship,
        ObjectKind::Expression => ObjectKindDto::Expression,
        ObjectKind::QueryGroup => ObjectKindDto::QueryGroup,
        ObjectKind::Role => ObjectKindDto::Role,
        ObjectKind::TablePermission => ObjectKindDto::TablePermission,
        ObjectKind::Culture => ObjectKindDto::Culture,
        ObjectKind::Perspective => ObjectKindDto::Perspective,
        ObjectKind::Report => ObjectKindDto::Report,
        ObjectKind::Page => ObjectKindDto::Page,
        ObjectKind::Visual => ObjectKindDto::Visual,
        ObjectKind::Filter => ObjectKindDto::Filter,
    }
}

fn table_kind_label(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Regular => "Table",
        TableKind::CalculatedTable => "Calculated table",
        TableKind::CalculationGroup => "Calculation group",
        TableKind::Remote => "Remote table (not available offline)",
    }
}

fn column_kind_label(kind: &ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Data { .. } => "Data column",
        ColumnKind::Calculated { .. } => "Calculated column",
        ColumnKind::CalculatedTableColumn { .. } => "Calculated table column",
        ColumnKind::RowNumber => "Row number (internal)",
    }
}

fn cardinality_label(c: RelationshipCardinality) -> &'static str {
    match c {
        RelationshipCardinality::ManyToOne => "Many to one (*:1)",
        RelationshipCardinality::OneToMany => "One to many (1:*)",
        RelationshipCardinality::OneToOne => "One to one (1:1)",
        RelationshipCardinality::ManyToMany => "Many to many (*:*)",
        RelationshipCardinality::Unknown => "Unknown",
    }
}

fn cross_filter_label(c: &CrossFilter) -> String {
    match c {
        CrossFilter::OneDirection => "Single".to_owned(),
        CrossFilter::BothDirections => "Both".to_owned(),
        CrossFilter::Automatic => "Automatic".to_owned(),
        CrossFilter::Unknown(raw) => raw.clone(),
    }
}

fn dataset_label(d: &DatasetReference) -> String {
    match d {
        DatasetReference::ByPath { path } => format!("byPath {path}"),
        DatasetReference::ByConnection { .. } => {
            format!("byConnection ({})", pbi_core::REMOTE_DESCRIPTION)
        }
        DatasetReference::Embedded => "Embedded model".to_owned(),
        DatasetReference::None => "None".to_owned(),
    }
}

fn relationships_of(model: Option<&Model>, table: &str) -> Vec<String> {
    model
        .map(|m| {
            m.relationships_for(table)
                .map(|r| {
                    format!(
                        "{} → {}{}",
                        dax_column_ref(&r.from.table, &r.from.column),
                        dax_column_ref(&r.to.table, &r.to.column),
                        if r.is_active { "" } else { " (inactive)" }
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn position_text(p: &pbi_model::Position) -> String {
    format!(
        "x {:.0}, y {:.0}, {:.0} × {:.0}, z {:.0}",
        p.x, p.y, p.width, p.height, p.z
    )
}

/// Everything about one object, for the Properties pane. `None` if `object` is not in the
/// document (stale selection from a previous document).
pub(crate) fn object_details(
    opened: &OpenedDocument,
    object: ObjectId,
) -> Option<ObjectDetailsDto> {
    use pbi_model::ObjectRef as R;

    let doc = &opened.document;
    let found = doc.find(object)?;
    let model = doc.model.as_ref();
    let mut s = Sheet::default();
    let mut parent = None;
    let mut name = found.name().to_owned();
    let mut description = non_empty(found.description());
    let (ann, ext, orig): (&[Annotation], Option<&PropertyBag>, Option<&Origin>) = match found {
        R::Model(m) => {
            s.opt("Culture", m.culture.as_deref());
            if let Some(level) = m.compatibility_level() {
                s.text("Compatibility level", level.to_string());
            }
            s.text("Tables", m.tables.len().to_string());
            s.text("Relationships", m.relationships.len().to_string());
            s.text("Expressions", m.expressions.len().to_string());
            s.text("Roles", m.roles.len().to_string());
            s.opt("Source query culture", m.source_query_culture.as_deref());
            s.flag(
                "Discourage implicit measures",
                m.discourage_implicit_measures,
            );
            if let Some(src) = &opened.model_source {
                s.mono("Stored as", src.kind());
                s.opt_mono("Folder", Some(src.folder()));
            }
            (&m.annotations, Some(&m.extra), m.origin.as_ref())
        }
        R::Database(d) => {
            if let Some(level) = d.compatibility_level {
                s.text("Compatibility level", level.to_string());
            }
            s.opt("Compatibility mode", d.compatibility_mode.as_deref());
            (&d.annotations, Some(&d.extra), d.origin.as_ref())
        }
        R::Table(t) => {
            let kind = t.kind();
            s.text("Type", table_kind_label(kind));
            s.mono("DAX reference", dax_table_ref(&t.name));
            s.text("Columns", t.columns.len().to_string());
            s.text("Measures", t.measures.len().to_string());
            s.text("Partitions", t.partitions.len().to_string());
            if !t.hierarchies.is_empty() {
                s.text("Hierarchies", t.hierarchies.len().to_string());
            }
            s.flag("Hidden", t.is_hidden);
            if t.is_private {
                s.flag("Private", true);
            }
            s.opt("Data category", t.data_category.as_deref());
            if let Some(mode) = t.partitions.first().map(|p| p.mode.as_str()) {
                s.mono("Storage mode", mode);
            }
            for rel in relationships_of(model, &t.name) {
                s.mono("Relationship", rel);
            }
            s.opt_mono("Lineage tag", t.lineage_tag.as_deref());
            if let Some(dax) = t.calculated_expression() {
                s.code("Calculated table (DAX)", CodeLanguage::Dax, dax);
            }
            for p in &t.partitions {
                if let PartitionSource::M { expression } = &p.source {
                    s.code(
                        format!("Partition {} (M)", p.name),
                        CodeLanguage::M,
                        expression,
                    );
                } else if let PartitionSource::Query { query, .. } = &p.source {
                    s.code(
                        format!("Partition {} (query)", p.name),
                        CodeLanguage::Text,
                        query,
                    );
                }
            }
            (&t.annotations, Some(&t.extra), t.origin.as_ref())
        }
        R::Column(t, c) => {
            parent = Some(t.name.clone());
            s.text("Type", column_kind_label(&c.kind));
            s.mono("DAX reference", dax_column_ref(&t.name, &c.name));
            s.mono("Data type", c.data_type.as_str());
            if let Some(src) = non_empty(c.source_column()) {
                let tooltip =
                    (src != c.name).then(|| "Renamed: differs from the display name".to_owned());
                s.push("Source column", src, true, tooltip);
            }
            s.opt_mono("Format string", c.format_string.as_deref());
            if let Some(by) = &c.summarize_by {
                s.mono("Summarize by", by.as_str());
            }
            s.opt("Sort by column", c.sort_by_column.as_deref());
            s.opt("Display folder", c.display_folder.as_deref());
            s.opt("Data category", c.data_category.as_deref());
            s.flag("Hidden", c.is_hidden);
            if c.is_key {
                s.flag("Key", true);
            }
            s.opt_mono("Lineage tag", c.lineage_tag.as_deref());
            if let Some(dax) = c.expression() {
                s.code("Expression (DAX)", CodeLanguage::Dax, dax);
            }
            (&c.annotations, Some(&c.extra), c.origin.as_ref())
        }
        R::Measure(t, m) => {
            parent = Some(t.name.clone());
            s.text("Type", "Measure");
            s.mono("DAX reference", dax_measure_ref(&m.name));
            s.text("Home table", t.name.clone());
            s.opt_mono("Format string", m.format_string.as_deref());
            s.opt("Display folder", m.display_folder.as_deref());
            s.opt("Data category", m.data_category.as_deref());
            s.flag("Hidden", m.is_hidden);
            s.opt_mono("Lineage tag", m.lineage_tag.as_deref());
            s.code("Expression (DAX)", CodeLanguage::Dax, &m.expression);
            (&m.annotations, Some(&m.extra), m.origin.as_ref())
        }
        R::Partition(t, p) => {
            parent = Some(t.name.clone());
            s.mono("Source type", p.source.kind_name());
            s.mono("Mode", p.mode.as_str());
            s.opt("Query group", p.query_group.as_deref());
            if let PartitionSource::Entity {
                entity_name,
                schema_name,
                ..
            } = &p.source
            {
                s.mono("Entity", entity_name.clone());
                s.opt_mono("Schema", schema_name.as_deref());
            }
            let language = match &p.source {
                PartitionSource::M { .. } => CodeLanguage::M,
                PartitionSource::Calculated { .. } => CodeLanguage::Dax,
                _ => CodeLanguage::Text,
            };
            if let Some(text) = p.source.expression() {
                s.code("Source", language, text);
            }
            if let PartitionSource::Unknown { props, .. } = &p.source {
                for prop in extra(props) {
                    s.properties.push(prop);
                }
            }
            (&p.annotations, Some(&p.extra), p.origin.as_ref())
        }
        R::Hierarchy(t, h) => {
            parent = Some(t.name.clone());
            let levels: Vec<&str> = h.levels.iter().map(|l| l.name.as_str()).collect();
            s.text("Levels", levels.join(" › "));
            s.flag("Hidden", h.is_hidden);
            s.opt_mono("Lineage tag", h.lineage_tag.as_deref());
            (&h.annotations, Some(&h.extra), h.origin.as_ref())
        }
        R::Level(t, h, l) => {
            parent = Some(format!("{} › {}", t.name, h.name));
            s.mono("Column", dax_column_ref(&t.name, &l.column));
            if let Some(ordinal) = l.ordinal {
                s.text("Ordinal", ordinal.to_string());
            }
            (&l.annotations, Some(&l.extra), None)
        }
        R::Relationship(r) => {
            name = format!(
                "{} → {}",
                dax_column_ref(&r.from.table, &r.from.column),
                dax_column_ref(&r.to.table, &r.to.column)
            );
            s.mono("From", dax_column_ref(&r.from.table, &r.from.column));
            s.mono("To", dax_column_ref(&r.to.table, &r.to.column));
            s.text("Cardinality", cardinality_label(cardinality(r)));
            s.text(
                "Cross filter",
                cross_filter_label(&r.cross_filtering_behavior),
            );
            s.flag("Active", r.is_active);
            s.mono("Security filtering", r.security_filtering_behavior.as_str());
            if r.rely_on_referential_integrity {
                s.flag("Assume referential integrity", true);
            }
            s.mono("Name", r.name.clone());
            (&r.annotations, Some(&r.extra), r.origin.as_ref())
        }
        R::Expression(e) => {
            s.text(
                "Type",
                if e.is_parameter() {
                    "Parameter"
                } else {
                    match e.kind {
                        ExpressionKind::Dax => "DAX expression",
                        _ => "M query",
                    }
                },
            );
            if let Some(t) = e.parameter_type() {
                s.mono("Parameter type", t);
            }
            for (k, v) in e.meta_record() {
                if !k.eq_ignore_ascii_case("Type") {
                    s.push(&k, v, true, Some("M metadata record".to_owned()));
                }
            }
            s.opt("Query group", e.query_group.as_deref());
            s.opt_mono("Lineage tag", e.lineage_tag.as_deref());
            s.code(
                "Expression",
                expression_language(&e.kind),
                &e.full_expression(),
            );
            (&e.annotations, Some(&e.extra), e.origin.as_ref())
        }
        R::QueryGroup(g) => {
            s.mono("Folder", g.folder.clone());
            (&g.annotations, Some(&g.extra), None)
        }
        R::Role(r) => {
            s.opt_mono("Model permission", r.model_permission.as_deref());
            for p in &r.table_permissions {
                if let Some(dax) = &p.filter_expression {
                    s.code(format!("{} filter (DAX)", p.table), CodeLanguage::Dax, dax);
                }
            }
            (&r.annotations, Some(&r.extra), r.origin.as_ref())
        }
        R::TablePermission(role, p) => {
            parent = Some(role.name.clone());
            s.text("Table", p.table.clone());
            if let Some(dax) = &p.filter_expression {
                s.code("Row filter (DAX)", CodeLanguage::Dax, dax);
            }
            (&p.annotations, Some(&p.extra), None)
        }
        R::Culture(c) => {
            if let Some(meta) = &c.linguistic_metadata {
                s.code("Linguistic metadata", CodeLanguage::Json, meta);
            }
            (&c.annotations, Some(&c.extra), None)
        }
        R::Perspective(p) => (&p.annotations, Some(&p.extra), None),
        R::Report(r) => {
            s.text("Dataset", dataset_label(&r.dataset_reference));
            s.text("Pages", r.pages.len().to_string());
            s.text(
                "Visuals",
                r.visuals()
                    .filter(|(_, v)| !v.is_group())
                    .count()
                    .to_string(),
            );
            s.text("Report filters", r.filters.len().to_string());
            if let Some(page) = r.start_page() {
                s.text("Start page", page.display_name.clone());
            }
            if let Some(theme) = &r.theme.base_theme {
                s.mono("Base theme", theme.name.clone());
            }
            if let Some(theme) = &r.theme.custom_theme {
                s.mono("Custom theme", theme.name.clone());
            }
            for (file, version) in &r.schema_versions {
                s.push(
                    "Schema version",
                    format!("{file} {version}"),
                    true,
                    Some("PBIR $schema versions vary between files".to_owned()),
                );
            }
            s.json("Settings (JSON)", &r.settings);
            (&r.annotations, Some(&r.extra), r.origin.as_ref())
        }
        R::Page(r, p) => {
            parent = Some(r.name.clone());
            if !p.display_name.trim().is_empty() {
                name = p.display_name.clone();
            }
            s.mono("Internal name", p.name.clone());
            s.text("Size", format!("{:.0} × {:.0}", p.width, p.height));
            s.mono("Display option", p.display_option.as_str());
            s.text(
                "Visuals",
                p.visuals
                    .iter()
                    .filter(|v| !v.is_group())
                    .count()
                    .to_string(),
            );
            s.text("Page filters", p.filters.len().to_string());
            s.flag("Hidden", p.is_hidden);
            s.json("Formatting (JSON)", &p.objects);
            (&p.annotations, Some(&p.extra), p.origin.as_ref())
        }
        R::Visual(r, p, v) => {
            parent = Some(format!("{} › {}", r.name, page_label(p)));
            name = visual_label(v);
            s.mono(
                "Visual type",
                if v.is_group() {
                    "group"
                } else {
                    v.visual_type.as_str()
                },
            );
            s.mono("Internal name", v.name.clone());
            s.text("Position", position_text(&v.position));
            if let Some(tab) = v.position.tab_order {
                s.text("Tab order", tab.to_string());
            }
            s.flag("Hidden", v.is_hidden);
            s.opt("Group", v.parent_group_name.as_deref());
            for p in &v.projections {
                let f = field(p);
                s.push(
                    &format!("Field · {}", f.role),
                    f.display_name,
                    false,
                    Some(format!("Source: {}", f.source_name)),
                );
            }
            for sort in &v.sort {
                s.mono(
                    "Sort",
                    format!("{} {:?}", format_field(&sort.field), sort.direction),
                );
            }
            for f in &v.filters {
                s.push(
                    "Filter",
                    filter_summary(f),
                    true,
                    f.field.as_ref().map(format_field),
                );
            }
            s.json("Formatting (JSON)", &v.objects);
            s.json("Container formatting (JSON)", &v.container_objects);
            (&v.annotations, Some(&v.extra), v.origin.as_ref())
        }
        R::Filter {
            report,
            page,
            visual,
            filter: f,
        } => {
            let scope = if visual.is_some() {
                "Visual"
            } else if page.is_some() {
                "Page"
            } else {
                "Report"
            };
            parent = Some(match (page, visual) {
                (Some(p), Some(v)) => {
                    format!("{} › {} › {}", report.name, page_label(p), visual_label(v))
                }
                (Some(p), None) => format!("{} › {}", report.name, page_label(p)),
                _ => report.name.clone(),
            });
            if let Some(display) = non_empty(f.display_name.as_deref()) {
                name = display;
            } else if let Some(field) = &f.field {
                name = format_field(field);
            }
            s.text("Scope", scope);
            if let Some(field) = &f.field {
                s.mono("Field", format_field(field));
            }
            s.mono("Type", f.kind.as_str());
            s.mono("Condition", filter_summary(f));
            s.flag("Hidden in view mode", f.is_hidden_in_view_mode);
            s.flag("Locked in view mode", f.is_locked_in_view_mode);
            s.opt_mono("How created", f.how_created.as_deref());
            s.mono("Internal name", f.name.clone());
            s.json("Definition (JSON)", &f.raw);
            (&[], None, f.origin.as_ref())
        }
    };
    if name.trim().is_empty() {
        name = format!("{:?}", found.kind());
    }
    if description.as_deref().is_some_and(str::is_empty) {
        description = None;
    }
    Some(ObjectDetailsDto {
        id: id(object),
        kind: object_kind(found.kind()),
        name,
        parent,
        description,
        properties: s.properties,
        code: s.code,
        annotations: annotations(ann),
        extra: ext.map(extra).unwrap_or_default(),
        origin: origin(orig),
    })
}

/// Title, else `type · first field` (e.g. `slicer · Region`), else the type.
fn visual_label(v: &Visual) -> String {
    if let Some(title) = visual_title(v) {
        return title;
    }
    let kind = if v.is_group() {
        "group"
    } else {
        v.visual_type.as_str()
    };
    match v.projections.first() {
        Some(p) => format!("{kind} · {}", p.label()),
        None if !kind.is_empty() => kind.to_owned(),
        None => v.name.clone(),
    }
}

fn page_label(p: &pbi_model::Page) -> &str {
    if p.display_name.trim().is_empty() {
        &p.name
    } else {
        &p.display_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::{AggregationFunction, ComparisonKind, FilterKind};

    fn channel() -> FieldExpr {
        FieldExpr::column("Stores", "Channel")
    }

    #[test]
    fn fields_use_dax_references() {
        assert_eq!(
            format_field(&FieldExpr::column("Web Sessions", "Device")),
            "'Web Sessions'[Device]"
        );
        assert_eq!(
            format_field(&FieldExpr::aggregation(
                FieldExpr::column("Orders", "Quantity"),
                AggregationFunction::Sum
            )),
            "Sum(Orders[Quantity])"
        );
    }

    #[test]
    fn projection_keeps_display_and_source_names() {
        let p = Projection {
            role: "Values".into(),
            field: FieldExpr::column("Products", "Product Name"),
            query_ref: "Products.Product Name".into(),
            display_name: Some("Product".into()),
            ..Projection::default()
        };
        let f = field(&p);
        assert_eq!(f.display_name, "Product");
        assert_eq!(f.source_name, "Products[Product Name]");
        assert_eq!(f.field.as_deref(), Some("Product Name"));
        assert!(f.renamed);
    }

    #[test]
    fn filter_summaries_read_like_dax() {
        let mut f = Filter {
            field: Some(channel()),
            kind: FilterKind::Categorical,
            condition: Some(Condition::In {
                exprs: vec![channel()],
                values: vec![vec![Literal::text("Retail")], vec![Literal::text("Outlet")]],
            }),
            ..Filter::default()
        };
        assert_eq!(
            filter_summary(&f),
            r#"Stores[Channel] IN {"Retail", "Outlet"}"#
        );
        f.condition = f.condition.map(|c| Condition::Not(Box::new(c)));
        assert!(filter_summary(&f).contains("NOT IN"));
        f.condition = Some(Condition::Comparison {
            kind: ComparisonKind::GreaterThanOrEqual,
            left: FieldExpr::column("T", "N"),
            right: FieldExpr::Literal(Literal::integer(5)),
        });
        assert_eq!(filter_summary(&f), "T[N] >= 5");
        f.condition = None;
        assert_eq!(filter_summary(&f), "All values (no condition)");
    }

    #[test]
    fn long_condition_chains_are_capped_without_deep_recursion() {
        let leaf = || Condition::Comparison {
            kind: ComparisonKind::Equal,
            left: FieldExpr::column("T", "C"),
            right: FieldExpr::Literal(Literal::integer(1)),
        };
        let mut chain = leaf();
        for _ in 0..50_000 {
            chain = Condition::And(Box::new(chain), Box::new(leaf()));
        }
        let f = Filter {
            condition: Some(chain),
            ..Filter::default()
        };
        let summary = filter_summary(&f);
        assert!(summary.starts_with("T[C] = 1 AND T[C] = 1"));
        assert!(summary.chars().count() <= MAX_SUMMARY_CHARS + 1);
        // Dropping a 50k-deep Box chain recursively would overflow the test thread's stack.
        if let Some(c) = f.condition {
            let mut stack = vec![c];
            while let Some(c) = stack.pop() {
                if let Condition::And(a, b) = c {
                    stack.push(*a);
                    stack.push(*b);
                }
            }
        }
    }

    #[test]
    fn visual_titles_come_from_title_groups_or_textboxes() {
        let mut v = Visual {
            visual_type: "pieChart".into(),
            container_objects: serde_json::json!({
                "title": [{"properties": {"text": {"expr": {"Literal": {"Value": "'Apps by Status'"}}}}}]
            }),
            ..Visual::default()
        };
        assert_eq!(visual_title(&v).as_deref(), Some("Apps by Status"));
        v.container_objects = Value::Null;
        v.visual_type = "textbox".into();
        v.objects = serde_json::json!({
            "general": [{"properties": {"paragraphs": [
                {"textRuns": [{"value": "Select filters"}]},
                {"textRuns": [{"value": "on the left"}]}
            ]}}]
        });
        assert_eq!(
            visual_title(&v).as_deref(),
            Some("Select filters on the left")
        );
        v.objects = Value::Null;
        assert_eq!(visual_title(&v), None);
    }
}
