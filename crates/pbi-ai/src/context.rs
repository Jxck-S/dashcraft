//! Building capability inputs from a model: metadata only (names, types,
//! descriptions, format strings, relationships, DAX), bounded; plus sample
//! rows when a caller asks for them.

use std::collections::BTreeSet;

use pbi_dax::Dependency;
use pbi_model::{Column, ColumnKind, CrossFilter, Measure, Model, Table, TableKind, names_eq};
use pbi_session::Session;
use serde_json::Value;

use crate::capability::{
    ColumnMeta, DaxExplainInput, DaxGenerateInput, MeasureMeta, ModelSchemaMeta, RelationshipMeta,
    SampleRows, TableMeta,
};
use crate::error::{AiError, truncate};

/// Longest expression or description sent.
pub const MAX_TEXT_CHARS: usize = 8000;
/// Most measures, columns, tables and relationships in one request.
pub const MAX_MEASURES: usize = 300;
pub const MAX_COLUMNS: usize = 1000;
pub const MAX_TABLES: usize = 200;
pub const MAX_RELATIONSHIPS: usize = 300;
/// Measure expressions are left out of a schema above this total.
const SCHEMA_EXPRESSION_BUDGET: usize = 30_000;

fn text(s: Option<&String>) -> Option<String> {
    s.map(|s| truncate(s.trim(), MAX_TEXT_CHARS))
        .filter(|s| !s.is_empty())
}

fn kind_str(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Regular => "regular",
        TableKind::CalculatedTable => "calculatedTable",
        TableKind::CalculationGroup => "calculationGroup",
        TableKind::Remote => "remote",
    }
}

pub(crate) fn table_meta(t: &Table) -> TableMeta {
    TableMeta {
        name: t.name.clone(),
        description: text(t.description.as_ref()),
        kind: kind_str(t.kind()).to_owned(),
        hidden: t.is_hidden,
        expression: t
            .calculated_expression()
            .map(|e| truncate(e.trim(), MAX_TEXT_CHARS)),
    }
}

pub(crate) fn column_meta(table: &str, c: &Column) -> ColumnMeta {
    ColumnMeta {
        table: table.to_owned(),
        name: c.name.clone(),
        data_type: c.data_type.as_str().to_owned(),
        description: text(c.description.as_ref()),
        format_string: c.format_string.clone(),
        expression: c.expression().map(|e| truncate(e.trim(), MAX_TEXT_CHARS)),
        sort_by_column: c.sort_by_column.clone(),
        hidden: c.is_hidden,
    }
}

pub(crate) fn measure_meta(table: &str, m: &Measure, with_expression: bool) -> MeasureMeta {
    MeasureMeta {
        table: table.to_owned(),
        name: m.name.clone(),
        expression: with_expression.then(|| truncate(m.expression.trim(), MAX_TEXT_CHARS)),
        format_string: m.format_string.clone(),
        description: text(m.description.as_ref()),
        display_folder: m.display_folder.clone(),
    }
}

fn relationship_meta(r: &pbi_model::Relationship) -> RelationshipMeta {
    RelationshipMeta {
        from: pbi_model::dax_column_ref(&r.from.table, &r.from.column),
        to: pbi_model::dax_column_ref(&r.to.table, &r.to.column),
        cardinality: format!(
            "{}-to-{}",
            r.from_cardinality.as_str(),
            r.to_cardinality.as_str()
        ),
        cross_filter: match r.cross_filtering_behavior {
            CrossFilter::OneDirection => "single".to_owned(),
            CrossFilter::BothDirections => "both".to_owned(),
            ref other => other.as_str().to_owned(),
        },
        active: r.is_active,
    }
}

/// Splits `Table[Name]`, `'Table'[Name]`, `[Name]` or `Name`.
fn split_measure_ref(spec: &str) -> (Option<String>, String) {
    let s = spec.trim();
    match s.find('[') {
        Some(i) if s.ends_with(']') => {
            let table = s
                .get(..i)
                .unwrap_or_default()
                .trim()
                .trim_matches('\'')
                .replace("''", "'");
            let name = s
                .get(i + 1..s.len() - 1)
                .unwrap_or_default()
                .replace("]]", "]");
            ((!table.is_empty()).then_some(table), name)
        }
        _ => (None, s.to_owned()),
    }
}

/// The measure `spec` (in `table` when given).
///
/// # Errors
/// [`AiError::NotFound`] with close names; [`AiError::InvalidInput`] for an
/// empty name.
pub(crate) fn find_measure<'m>(
    model: &'m Model,
    spec: &str,
    table: Option<&str>,
) -> Result<(&'m Table, &'m Measure), AiError> {
    let (table_part, name) = split_measure_ref(spec);
    if name.trim().is_empty() {
        return Err(AiError::InvalidInput("the measure name is empty".into()));
    }
    let table = table
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .or(table_part);
    let found = model.tables.iter().find_map(|t| {
        let table_ok = table.as_deref().is_none_or(|w| names_eq(w, &t.name));
        t.measures
            .iter()
            .find(|m| table_ok && names_eq(&m.name, &name))
            .map(|m| (t, m))
    });
    found.ok_or_else(|| {
        let needle = name.to_lowercase();
        let all: Vec<String> = model
            .tables
            .iter()
            .flat_map(|t| t.measures.iter().map(|m| m.name.clone()))
            .collect();
        let mut candidates: Vec<String> = all
            .iter()
            .filter(|m| m.to_lowercase().contains(&needle) || needle.contains(&m.to_lowercase()))
            .take(10)
            .cloned()
            .collect();
        if candidates.is_empty() {
            candidates = all.into_iter().take(10).collect();
        }
        AiError::NotFound {
            message: format!(
                "no measure '{name}'{}",
                table
                    .map(|t| format!(" in table '{t}'"))
                    .unwrap_or_default()
            ),
            candidates,
        }
    })
}

/// The `dax.explain` input for `measure` of `table`: the measure, the
/// measures it reaches, their columns and tables, and the relationships
/// touching those tables.
pub(crate) fn explain_input(model: &Model, table: &Table, measure: &Measure) -> DaxExplainInput {
    let mut notes = Vec::new();
    let deps = match pbi_dax::parse_expression(&measure.expression) {
        Ok(_) => pbi_dax::transitive_dependencies(model, Some(&table.name), &measure.expression)
            .unwrap_or_else(|e| {
                notes.push(format!("dependencies could not be resolved: {e}"));
                Vec::new()
            }),
        Err(e) => {
            notes.push(format!(
                "the measure's DAX does not parse: {}",
                e.render(&measure.expression)
            ));
            Vec::new()
        }
    };
    let mut tables: Vec<String> = vec![table.name.clone()];
    let mut referenced_measures = Vec::new();
    let mut columns = Vec::new();
    for dep in &deps {
        match dep {
            Dependency::Measure(name) => {
                if let Some((t, m)) = model.find_measure(name)
                    && !(names_eq(&t.name, &table.name) && names_eq(&m.name, &measure.name))
                    && referenced_measures.len() < MAX_MEASURES
                {
                    add_table(&mut tables, &t.name);
                    referenced_measures.push(measure_meta(&t.name, m, true));
                }
            }
            Dependency::Column { table: t, column } => {
                if let Some(home) = model.table(t)
                    && let Some(c) = home.column(column)
                    && columns.len() < MAX_COLUMNS
                {
                    add_table(&mut tables, &home.name);
                    columns.push(column_meta(&home.name, c));
                }
            }
            Dependency::Table(t) => {
                if let Some(home) = model.table(t) {
                    add_table(&mut tables, &home.name);
                }
            }
        }
    }
    tables.truncate(MAX_TABLES);
    let relationships = model
        .relationships
        .iter()
        .filter(|r| tables.iter().any(|t| r.involves(t)))
        .take(MAX_RELATIONSHIPS)
        .map(relationship_meta)
        .collect();
    DaxExplainInput {
        measure: measure_meta(&table.name, measure, true),
        referenced_measures,
        columns,
        tables: tables
            .iter()
            .filter_map(|t| model.table(t))
            .map(table_meta)
            .collect(),
        relationships,
        culture: model.culture.clone(),
        notes,
    }
}

fn add_table(tables: &mut Vec<String>, name: &str) {
    if !tables.iter().any(|t| names_eq(t, name)) {
        tables.push(name.to_owned());
    }
}

/// Every table, column, measure and relationship of `model` (bounded;
/// measure expressions only while they fit a budget).
pub(crate) fn schema_meta(model: &Model) -> ModelSchemaMeta {
    let mut truncated = false;
    let tables: Vec<&Table> = model.tables.iter().filter(|t| !t.is_private).collect();
    if tables.len() > MAX_TABLES {
        truncated = true;
    }
    let tables: Vec<&Table> = tables.into_iter().take(MAX_TABLES).collect();
    let mut columns = Vec::new();
    for t in &tables {
        for c in &t.columns {
            if matches!(c.kind, ColumnKind::RowNumber) {
                continue;
            }
            if columns.len() >= MAX_COLUMNS {
                truncated = true;
                break;
            }
            columns.push(column_meta(&t.name, c));
        }
    }
    let all_measures: Vec<(&Table, &Measure)> = tables
        .iter()
        .flat_map(|t| t.measures.iter().map(move |m| (*t, m)))
        .collect();
    if all_measures.len() > MAX_MEASURES {
        truncated = true;
    }
    let expression_chars: usize = all_measures
        .iter()
        .take(MAX_MEASURES)
        .map(|(_, m)| m.expression.len())
        .sum();
    let with_expressions = expression_chars <= SCHEMA_EXPRESSION_BUDGET;
    if !with_expressions {
        truncated = true;
    }
    let measures = all_measures
        .iter()
        .take(MAX_MEASURES)
        .map(|(t, m)| measure_meta(&t.name, m, with_expressions))
        .collect();
    if model.relationships.len() > MAX_RELATIONSHIPS {
        truncated = true;
    }
    ModelSchemaMeta {
        tables: tables.iter().map(|t| table_meta(t)).collect(),
        columns,
        measures,
        relationships: model
            .relationships
            .iter()
            .take(MAX_RELATIONSHIPS)
            .map(relationship_meta)
            .collect(),
        truncated,
    }
}

/// The `dax.generate` input.
pub(crate) fn generate_input(
    model: &Model,
    request: &str,
    target_table: Option<&str>,
    sample: Option<SampleRows>,
) -> DaxGenerateInput {
    DaxGenerateInput {
        request: request.trim().to_owned(),
        target_table: target_table.map(str::to_owned),
        schema: schema_meta(model),
        culture: model.culture.clone(),
        sample,
    }
}

/// The first `rows` rows of `table`'s loaded data.
///
/// # Errors
/// [`AiError::NotFound`] for an unknown table.
pub(crate) fn sample_rows(
    session: &Session,
    table: &str,
    rows: usize,
) -> Result<SampleRows, AiError> {
    let batch = session.table_batch(table).map_err(|e| AiError::NotFound {
        message: e.to_string(),
        candidates: Vec::new(),
    })?;
    let schema = batch.schema();
    let keep: Vec<(usize, String)> = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| f.name() != pbi_engine::ROW_ID_COLUMN)
        .map(|(i, f)| (i, f.name().clone()))
        .collect();
    let n = rows.min(batch.num_rows());
    let mut out = Vec::with_capacity(n);
    for r in 0..n {
        let row = keep
            .iter()
            .map(|(i, _)| {
                pbi_engine::value::value_from_array(batch.column(*i), r)
                    .map(|v| pbi_session::value_to_json(&v))
                    .unwrap_or(Value::Null)
            })
            .map(|v| match v {
                Value::String(s) => Value::String(truncate(&s, 500)),
                other => other,
            })
            .collect();
        out.push(row);
    }
    Ok(SampleRows {
        table: table.to_owned(),
        columns: keep.into_iter().map(|(_, n)| n).collect(),
        rows: out,
    })
}

/// Tables in a model, for "did you mean".
pub(crate) fn table_names(model: &Model) -> Vec<String> {
    model.tables.iter().map(|t| t.name.clone()).collect()
}

/// Names of the referenced objects of an explain input, for the result.
pub(crate) fn reference_names(input: &DaxExplainInput) -> (Vec<String>, Vec<String>, Vec<String>) {
    let measures = input
        .referenced_measures
        .iter()
        .map(|m| pbi_model::dax_column_ref(&m.table, &m.name))
        .collect();
    let columns = input
        .columns
        .iter()
        .map(|c| {
            format!(
                "{} ({})",
                pbi_model::dax_column_ref(&c.table, &c.name),
                c.data_type
            )
        })
        .collect();
    let tables: BTreeSet<String> = input.tables.iter().map(|t| t.name.clone()).collect();
    (measures, columns, tables.into_iter().collect())
}
