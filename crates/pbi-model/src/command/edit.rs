//! Single-property edits.

use std::mem;

use super::{Applied, ChangeEvent, Command, CommandError, expect_kind, model_mut};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::report::Position;
use crate::semantic::{Model, Table};

fn changed(id: ObjectId, differs: bool) -> Vec<ChangeEvent> {
    if differs {
        vec![ChangeEvent::ObjectChanged { id }]
    } else {
        Vec::new()
    }
}

/// Replaces a measure's DAX expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetMeasureExpression {
    pub measure: ObjectId,
    pub expression: String,
}

impl Command for SetMeasureExpression {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Measure { table, measure } =
            expect_kind(doc, self.measure, ObjectKind::Measure)?
        else {
            return Err(CommandError::NotFound(self.measure));
        };
        let slot = model_mut(doc)?
            .tables
            .get_mut(table)
            .and_then(|t| t.measures.get_mut(measure))
            .ok_or(CommandError::NotFound(self.measure))?;
        let old = mem::replace(&mut slot.expression, self.expression.clone());
        let events = changed(self.measure, old != self.expression);
        Ok(Applied::new(
            SetMeasureExpression {
                measure: self.measure,
                expression: old,
            },
            events,
        ))
    }

    fn label(&self) -> String {
        "Edit measure expression".to_owned()
    }
}

/// Sets (or clears, with `None`) the description of any object that has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetDescription {
    pub id: ObjectId,
    pub text: Option<String>,
}

fn table(model: &mut Model, index: usize) -> Option<&mut Table> {
    model.tables.get_mut(index)
}

fn description_slot(doc: &mut Document, id: ObjectId) -> Result<&mut Option<String>, CommandError> {
    let path = doc.locate(id).ok_or(CommandError::NotFound(id))?;
    let kind = path.kind();
    let missing = || CommandError::NotFound(id);
    let unsupported = CommandError::Unsupported {
        kind,
        operation: "descriptions",
    };
    let model = match kind {
        ObjectKind::Report
        | ObjectKind::Page
        | ObjectKind::Visual
        | ObjectKind::Filter
        | ObjectKind::Relationship
        | ObjectKind::TablePermission
        | ObjectKind::Culture => return Err(unsupported),
        _ => model_mut(doc)?,
    };
    Ok(match path {
        ObjectPath::Model => &mut model.description,
        ObjectPath::Database => &mut model.database.as_mut().ok_or_else(missing)?.description,
        ObjectPath::Table { table: t } => &mut table(model, t).ok_or_else(missing)?.description,
        ObjectPath::Column { table: t, column } => {
            &mut table(model, t)
                .and_then(|t| t.columns.get_mut(column))
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::Measure { table: t, measure } => {
            &mut table(model, t)
                .and_then(|t| t.measures.get_mut(measure))
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::Partition {
            table: t,
            partition,
        } => {
            &mut table(model, t)
                .and_then(|t| t.partitions.get_mut(partition))
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::Hierarchy {
            table: t,
            hierarchy,
        } => {
            &mut table(model, t)
                .and_then(|t| t.hierarchies.get_mut(hierarchy))
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::Level {
            table: t,
            hierarchy,
            level,
        } => {
            &mut table(model, t)
                .and_then(|t| t.hierarchies.get_mut(hierarchy))
                .and_then(|h| h.levels.get_mut(level))
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::Expression { expression } => {
            &mut model
                .expressions
                .get_mut(expression)
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::QueryGroup { group } => {
            &mut model
                .query_groups
                .get_mut(group)
                .ok_or_else(missing)?
                .description
        }
        ObjectPath::Role { role } => {
            &mut model.roles.get_mut(role).ok_or_else(missing)?.description
        }
        ObjectPath::Perspective { perspective } => {
            &mut model
                .perspectives
                .get_mut(perspective)
                .ok_or_else(missing)?
                .description
        }
        _ => return Err(unsupported),
    })
}

impl Command for SetDescription {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let slot = description_slot(doc, self.id)?;
        let old = mem::replace(slot, self.text.clone());
        let events = changed(self.id, old != self.text);
        Ok(Applied::new(
            SetDescription {
                id: self.id,
                text: old,
            },
            events,
        ))
    }

    fn label(&self) -> String {
        "Edit description".to_owned()
    }
}

/// Moves/resizes a visual.
#[derive(Debug, Clone, PartialEq)]
pub struct SetVisualPosition {
    pub visual: ObjectId,
    pub position: Position,
}

impl Command for SetVisualPosition {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Visual {
            report,
            page,
            visual,
        } = expect_kind(doc, self.visual, ObjectKind::Visual)?
        else {
            return Err(CommandError::NotFound(self.visual));
        };
        let slot = doc
            .reports
            .get_mut(report)
            .and_then(|r| r.pages.get_mut(page))
            .and_then(|p| p.visuals.get_mut(visual))
            .ok_or(CommandError::NotFound(self.visual))?;
        let old = mem::replace(&mut slot.position, self.position.clone());
        let events = changed(self.visual, old != self.position);
        Ok(Applied::new(
            SetVisualPosition {
                visual: self.visual,
                position: old,
            },
            events,
        ))
    }

    fn label(&self) -> String {
        "Move visual".to_owned()
    }
}
