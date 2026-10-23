//! Rename commands. They rename the object and rewrite structured references
//! to it (see [`crate::rewrite`]); the inverse restores every touched object
//! exactly (via [`ReplaceObjects`]).

use super::snapshot::{ReplaceObjects, Snapshot};
use super::{Applied, Command, CommandError, expect_kind, model};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::names::{is_valid_object_name, names_eq};
use crate::rewrite::Rename;

fn validate_name(name: &str) -> Result<(), CommandError> {
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

/// Snapshots of every report part whose field references change.
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

fn no_op(inverse: impl Command + 'static) -> Applied {
    Applied::new(inverse, Vec::new())
}

/// Renames a table and updates relationships, role table permissions and
/// report field references.
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

/// Renames a measure and updates report field references.
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
        validate_name(&self.new_name)?;
        let model = model(doc)?;
        let table = model
            .tables
            .get(t)
            .ok_or(CommandError::NotFound(self.measure))?;
        let measure = table
            .measures
            .get(m)
            .ok_or(CommandError::NotFound(self.measure))?;
        // Measure names are unique across the model.
        for other_table in &model.tables {
            if let Some(other) = other_table
                .measures
                .iter()
                .find(|x| x.id != measure.id && names_eq(&x.name, &self.new_name))
            {
                return Err(conflict(
                    &self.new_name,
                    format!("measure '{}'[{}]", other_table.name, other.name),
                ));
            }
        }
        if let Some(other) = table.column(&self.new_name) {
            return Err(conflict(
                &self.new_name,
                format!("column '{}'[{}]", table.name, other.name),
            ));
        }
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
