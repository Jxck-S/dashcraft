//! Rename commands. They rename the object and rewrite structured references
//! to it (see [`crate::rewrite`]); the inverse restores every touched object
//! exactly (via [`ReplaceObjects`]). Renaming a table or a shared expression
//! also renames it in the model's `PBI_QueryOrder` annotation (Power Query's
//! query order); DAX and M text are rewritten by `pbi-session`.

use super::snapshot::{ReplaceObjects, Snapshot};
use super::{Applied, Command, CommandError, expect_kind, model};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::names::{is_valid_object_name, names_eq};
use crate::rewrite::Rename;
use crate::semantic::Model;

pub(super) fn validate_name(name: &str) -> Result<(), CommandError> {
    if is_valid_object_name(name) {
        Ok(())
    } else {
        Err(CommandError::InvalidName(name.to_owned()))
    }
}

fn conflict(name: &str, existing: String) -> CommandError {
    CommandError::NameConflict {
        name: name.to_owned(),
        existing,
    }
}

/// Checks that measure `id` (unassigned for a new one) of table `table` can
/// be named `name`: measure names are unique across the model and the
/// report-level measures, and differ from the table's column names.
pub(super) fn check_measure_name(
    doc: &Document,
    table: usize,
    id: ObjectId,
    name: &str,
) -> Result<(), CommandError> {
    validate_name(name)?;
    let model = model(doc)?;
    for other_table in &model.tables {
        if let Some(other) = other_table
            .measures
            .iter()
            .find(|x| x.id != id && names_eq(&x.name, name))
        {
            return Err(conflict(
                name,
                format!("measure '{}'[{}]", other_table.name, other.name),
            ));
        }
    }
    if let Some(t) = model.tables.get(table)
        && let Some(other) = t.column(name)
    {
        return Err(conflict(
            name,
            format!("column '{}'[{}]", t.name, other.name),
        ));
    }
    check_report_measures(doc, None, name)
}

/// Refuses `name` if a report-level measure has it (one shown in `table`
/// only, when given: a column cannot share its table with a measure of the
/// same name).
pub(super) fn check_report_measures(
    doc: &Document,
    table: Option<&str>,
    name: &str,
) -> Result<(), CommandError> {
    for report in &doc.reports {
        if let Some(m) = report.measure(name)
            && table.is_none_or(|t| names_eq(&m.table, t))
        {
            return Err(conflict(
                name,
                format!(
                    "report-level measure '{}'[{}] of report '{}'",
                    m.table, m.name, report.name
                ),
            ));
        }
    }
    Ok(())
}

/// Snapshots of every report part whose field references change (report
/// filters and measures, page filters, visuals).
fn report_snapshots(doc: &Document, rename: &Rename) -> Vec<Snapshot> {
    let mut out = Vec::new();
    for report in &doc.reports {
        let mut filters = report.filters.clone();
        if rename.apply_to_filters(&mut filters) {
            out.push(Snapshot::ReportFilters {
                report: report.id,
                filters,
            });
        }
        let mut measures = report.measures.clone();
        if rename.apply_to_report_measures(&mut measures) {
            out.push(Snapshot::ReportMeasures {
                report: report.id,
                measures,
            });
        }
        for page in &report.pages {
            let mut filters = page.filters.clone();
            if rename.apply_to_filters(&mut filters) {
                out.push(Snapshot::PageFilters {
                    page: page.id,
                    filters,
                });
            }
            for visual in &page.visuals {
                let mut v = visual.clone();
                if rename.apply_to_visual(&mut v) {
                    out.push(Snapshot::Visual(v));
                }
            }
        }
    }
    out
}

/// Annotation listing the Power Query queries in their display order (a
/// JSON list of names).
const QUERY_ORDER: &str = "PBI_QueryOrder";

/// Whether the table is a Power Query query (has an M partition).
fn is_query(table: &crate::semantic::Table) -> bool {
    table
        .partitions
        .iter()
        .any(|p| matches!(p.source, crate::semantic::PartitionSource::M { .. }))
}

/// Queries share one namespace in M: a table with an M partition or a shared
/// expression other than `id` that is already named `name`.
fn query_name_conflict(model: &Model, id: ObjectId, name: &str) -> Option<CommandError> {
    if let Some(e) = model
        .expressions
        .iter()
        .find(|e| e.id != id && names_eq(&e.name, name))
    {
        return Some(conflict(name, format!("query '{}'", e.name)));
    }
    model
        .tables
        .iter()
        .find(|t| t.id != id && is_query(t) && names_eq(&t.name, name))
        .map(|t| conflict(name, format!("query (table) '{}'", t.name)))
}

/// The model's annotations with query `old` renamed to `new` in
/// `PBI_QueryOrder`, if it lists `old`.
fn query_order(model: &Model, old: &str, new: &str) -> Option<Snapshot> {
    let index = model
        .annotations
        .iter()
        .position(|a| a.name == QUERY_ORDER)?;
    let order = model.annotations.get(index)?;
    let names: Vec<String> = serde_json::from_str(&order.value).ok()?;
    if !names.iter().any(|n| n == old) {
        return None;
    }
    let renamed: Vec<String> = names
        .into_iter()
        .map(|n| if n == old { new.to_owned() } else { n })
        .collect();
    let mut annotations = model.annotations.clone();
    annotations.get_mut(index)?.value = serde_json::to_string(&renamed).ok()?;
    Some(Snapshot::ModelAnnotations {
        model: model.id,
        annotations,
    })
}

fn no_op(inverse: impl Command + 'static) -> Applied {
    Applied::new(inverse, Vec::new())
}

/// Renames a table and updates relationships, role table permissions, the
/// query order, report field references and report-level measures (the
/// table they are shown in, their measure references).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTable {
    pub table: ObjectId,
    pub new_name: String,
}

impl Command for RenameTable {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Table { table: index } = expect_kind(doc, self.table, ObjectKind::Table)?
        else {
            return Err(CommandError::NotFound(self.table));
        };
        validate_name(&self.new_name)?;
        let model = model(doc)?;
        let table = model
            .tables
            .get(index)
            .ok_or(CommandError::NotFound(self.table))?;
        if let Some(other) = model
            .tables
            .iter()
            .find(|t| t.id != table.id && names_eq(&t.name, &self.new_name))
        {
            return Err(conflict(&self.new_name, format!("table '{}'", other.name)));
        }
        if is_query(table)
            && let Some(err) = query_name_conflict(model, table.id, &self.new_name)
        {
            return Err(err);
        }
        let old = table.name.clone();
        if old == self.new_name {
            return Ok(no_op(self.clone()));
        }
        let rename = Rename::Table {
            old: old.clone(),
            new: self.new_name.clone(),
        };

        let mut renamed = table.clone();
        renamed.name.clone_from(&self.new_name);
        let mut items = vec![Snapshot::Table(renamed)];
        for r in &model.relationships {
            let mut r = r.clone();
            if rename.apply_to_relationship(&mut r) {
                items.push(Snapshot::Relationship(r));
            }
        }
        for role in &model.roles {
            let mut role = role.clone();
            let mut changed = false;
            for p in &mut role.table_permissions {
                if names_eq(&p.table, &old) && p.table != self.new_name {
                    p.table.clone_from(&self.new_name);
                    changed = true;
                }
            }
            if changed {
                items.push(Snapshot::Role(role));
            }
        }
        items.extend(query_order(model, &old, &self.new_name));
        items.extend(report_snapshots(doc, &rename));
        ReplaceObjects {
            label: self.label(),
            items,
        }
        .apply(doc)
    }

    fn label(&self) -> String {
        format!("Rename table to '{}'", self.new_name)
    }
}

/// Renames a shared expression (an M query, parameter or function) and
/// updates the entity partitions sourced from it (`expressionSource`) and
/// the query order. Names are unique among queries: shared expressions and
/// tables with an M partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameExpression {
    pub expression: ObjectId,
    pub new_name: String,
}

impl Command for RenameExpression {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Expression { expression: index } =
            expect_kind(doc, self.expression, ObjectKind::Expression)?
        else {
            return Err(CommandError::NotFound(self.expression));
        };
        validate_name(&self.new_name)?;
        let model = model(doc)?;
        let expression = model
            .expressions
            .get(index)
            .ok_or(CommandError::NotFound(self.expression))?;
        if let Some(err) = query_name_conflict(model, expression.id, &self.new_name) {
            return Err(err);
        }
        let old = expression.name.clone();
        if old == self.new_name {
            return Ok(no_op(self.clone()));
        }
        let mut renamed = expression.clone();
        renamed.name.clone_from(&self.new_name);
        let mut items = vec![Snapshot::Expression(renamed)];
        for table in &model.tables {
            let mut t = table.clone();
            let mut changed = false;
            for p in &mut t.partitions {
                if let crate::semantic::PartitionSource::Entity {
                    expression_source: Some(source),
                    ..
                } = &mut p.source
                    && names_eq(source, &old)
                {
                    source.clone_from(&self.new_name);
                    changed = true;
                }
            }
            if changed {
                items.push(Snapshot::Table(t));
            }
        }
        items.extend(query_order(model, &old, &self.new_name));
        ReplaceObjects {
            label: self.label(),
            items,
        }
        .apply(doc)
    }

    fn label(&self) -> String {
        format!("Rename query to '{}'", self.new_name)
    }
}

/// Renames a column and updates sort-by columns, hierarchy levels,
/// relationships and report field references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameColumn {
    pub column: ObjectId,
    pub new_name: String,
}

impl Command for RenameColumn {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Column {
            table: t,
            column: c,
        } = expect_kind(doc, self.column, ObjectKind::Column)?
        else {
            return Err(CommandError::NotFound(self.column));
        };
        validate_name(&self.new_name)?;
        let model = model(doc)?;
        let table = model
            .tables
            .get(t)
            .ok_or(CommandError::NotFound(self.column))?;
        let column = table
            .columns
            .get(c)
            .ok_or(CommandError::NotFound(self.column))?;
        if let Some(other) = table
            .columns
            .iter()
            .find(|x| x.id != column.id && names_eq(&x.name, &self.new_name))
        {
            return Err(conflict(
                &self.new_name,
                format!("column '{}'[{}]", table.name, other.name),
            ));
        }
        if let Some(other) = table.measure(&self.new_name) {
            return Err(conflict(
                &self.new_name,
                format!("measure '{}'[{}]", table.name, other.name),
            ));
        }
        check_report_measures(doc, Some(&table.name), &self.new_name)?;
        let old = column.name.clone();
        if old == self.new_name {
            return Ok(no_op(self.clone()));
        }
        let rename = Rename::Column {
            table: table.name.clone(),
            old: old.clone(),
            new: self.new_name.clone(),
        };

        let mut new_table = table.clone();
        for (i, col) in new_table.columns.iter_mut().enumerate() {
            if i == c {
                col.name.clone_from(&self.new_name);
            }
            if let Some(sort_by) = &mut col.sort_by_column
                && names_eq(sort_by, &old)
            {
                sort_by.clone_from(&self.new_name);
            }
        }
        for level in new_table
            .hierarchies
            .iter_mut()
            .flat_map(|h| h.levels.iter_mut())
        {
            if names_eq(&level.column, &old) {
                level.column.clone_from(&self.new_name);
            }
        }
        let mut items = vec![Snapshot::Table(new_table)];
        for r in &model.relationships {
            let mut r = r.clone();
            if rename.apply_to_relationship(&mut r) {
                items.push(Snapshot::Relationship(r));
            }
        }
        items.extend(report_snapshots(doc, &rename));
        ReplaceObjects {
            label: self.label(),
            items,
        }
        .apply(doc)
    }

    fn label(&self) -> String {
        format!("Rename column to '{}'", self.new_name)
    }
}

/// Renames a measure and updates report field references and the measure
/// references of report-level measures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameMeasure {
    pub measure: ObjectId,
    pub new_name: String,
}

impl Command for RenameMeasure {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Measure {
            table: t,
            measure: m,
        } = expect_kind(doc, self.measure, ObjectKind::Measure)?
        else {
            return Err(CommandError::NotFound(self.measure));
        };
        let model = model(doc)?;
        let table = model
            .tables
            .get(t)
            .ok_or(CommandError::NotFound(self.measure))?;
        let measure = table
            .measures
            .get(m)
            .ok_or(CommandError::NotFound(self.measure))?;
        check_measure_name(doc, t, measure.id, &self.new_name)?;
        let old = measure.name.clone();
        if old == self.new_name {
            return Ok(no_op(self.clone()));
        }
        let rename = Rename::Measure {
            table: table.name.clone(),
            old,
            new: self.new_name.clone(),
        };
        let mut renamed = measure.clone();
        renamed.name.clone_from(&self.new_name);
        let mut items = vec![Snapshot::Measure(renamed)];
        items.extend(report_snapshots(doc, &rename));
        ReplaceObjects {
            label: self.label(),
            items,
        }
        .apply(doc)
    }

    fn label(&self) -> String {
        format!("Rename measure to '{}'", self.new_name)
    }
}
