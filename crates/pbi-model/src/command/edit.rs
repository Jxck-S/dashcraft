//! Single-property edits.

use std::mem;

use super::{Applied, ChangeEvent, Command, CommandError, CommandGroup, expect_kind, model_mut};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
#[cfg(doc)]
use crate::report::Page;
use crate::report::Position;
use crate::semantic::{ColumnKind, DataType, Model, SummarizeBy, Table};

pub(super) fn changed(id: ObjectId, differs: bool) -> Vec<ChangeEvent> {
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

/// Patches a measure's properties in one step. `None` leaves a property
/// unchanged; `Some(None)` clears an optional one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetMeasureProperties {
    pub measure: ObjectId,
    pub expression: Option<String>,
    pub format_string: Option<Option<String>>,
    pub display_folder: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub is_hidden: Option<bool>,
}

pub(super) fn swap<T: PartialEq + Clone>(
    slot: &mut T,
    new: &Option<T>,
    differs: &mut bool,
) -> Option<T> {
    let new = new.as_ref()?;
    *differs |= slot != new;
    Some(mem::replace(slot, new.clone()))
}

impl Command for SetMeasureProperties {
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
        if matches!(self.format_string, Some(Some(_))) && slot.format_string_definition().is_some()
        {
            return Err(CommandError::Rejected(format!(
                "measure '{}' has a dynamic format string; a static format string cannot be set too",
                slot.name
            )));
        }
        let mut differs = false;
        let inverse = SetMeasureProperties {
            measure: self.measure,
            expression: swap(&mut slot.expression, &self.expression, &mut differs),
            format_string: swap(&mut slot.format_string, &self.format_string, &mut differs),
            display_folder: swap(&mut slot.display_folder, &self.display_folder, &mut differs),
            description: swap(&mut slot.description, &self.description, &mut differs),
            is_hidden: swap(&mut slot.is_hidden, &self.is_hidden, &mut differs),
        };
        Ok(Applied::new(inverse, changed(self.measure, differs)))
    }

    fn label(&self) -> String {
        "Edit measure".to_owned()
    }
}

/// Patches a column's properties in one step. `None` leaves a property
/// unchanged; `Some(None)` clears an optional one. `expression` and
/// `data_type` apply to calculated columns only (a data column's values and
/// type come from its query).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetColumnProperties {
    pub column: ObjectId,
    pub expression: Option<String>,
    pub data_type: Option<DataType>,
    pub summarize_by: Option<Option<SummarizeBy>>,
    pub format_string: Option<Option<String>>,
    pub display_folder: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub is_hidden: Option<bool>,
}

impl Command for SetColumnProperties {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Column { table, column } =
            expect_kind(doc, self.column, ObjectKind::Column)?
        else {
            return Err(CommandError::NotFound(self.column));
        };
        let slot = model_mut(doc)?
            .tables
            .get_mut(table)
            .and_then(|t| t.columns.get_mut(column))
            .ok_or(CommandError::NotFound(self.column))?;
        if (self.expression.is_some() || self.data_type.is_some()) && !slot.is_calculated() {
            return Err(CommandError::Rejected(format!(
                "column [{}] is not a calculated column: its values and data type come from its query",
                slot.name
            )));
        }
        let mut differs = false;
        let expression = match (&mut slot.kind, &self.expression) {
            (ColumnKind::Calculated { expression }, Some(new)) => {
                differs |= expression != new;
                Some(mem::replace(expression, new.clone()))
            }
            _ => None,
        };
        let inverse = SetColumnProperties {
            column: self.column,
            expression,
            data_type: swap(&mut slot.data_type, &self.data_type, &mut differs),
            summarize_by: swap(&mut slot.summarize_by, &self.summarize_by, &mut differs),
            format_string: swap(&mut slot.format_string, &self.format_string, &mut differs),
            display_folder: swap(&mut slot.display_folder, &self.display_folder, &mut differs),
            description: swap(&mut slot.description, &self.description, &mut differs),
            is_hidden: swap(&mut slot.is_hidden, &self.is_hidden, &mut differs),
        };
        Ok(Applied::new(inverse, changed(self.column, differs)))
    }

    fn label(&self) -> String {
        "Edit column".to_owned()
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

/// Moves and/or resizes a visual or visual group to the given values (the
/// others are kept), evaluated against the document it is applied to. A
/// group takes the visuals in it along (see [`Page::placement_of`]); the
/// inverse restores every changed position exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveVisual {
    pub visual: ObjectId,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl Command for MoveVisual {
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
            .ok_or(CommandError::NotFound(self.visual))?;
        let mut to = slot
            .visuals
            .get(visual)
            .ok_or(CommandError::NotFound(self.visual))?
            .position
            .clone();
        for (name, value, target) in [("x", self.x, &mut to.x), ("y", self.y, &mut to.y)] {
            if let Some(v) = value {
                if !v.is_finite() {
                    return Err(CommandError::Rejected(format!("{name} must be finite")));
                }
                *target = v;
            }
        }
        for (name, value, target) in [
            ("width", self.width, &mut to.width),
            ("height", self.height, &mut to.height),
        ] {
            if let Some(v) = value {
                if !(v.is_finite() && v > 0.0) {
                    return Err(CommandError::Rejected(format!(
                        "{name} must be a positive number"
                    )));
                }
                *target = v;
            }
        }
        let mut inverse = CommandGroup::new(self.label());
        let mut events = Vec::new();
        for (i, position) in slot.placement_of(visual, &to) {
            let Some(v) = slot.visuals.get_mut(i) else {
                continue;
            };
            let old = mem::replace(&mut v.position, position);
            if old != v.position {
                events.push(ChangeEvent::ObjectChanged { id: v.id });
            }
            inverse = inverse.with(SetVisualPosition {
                visual: v.id,
                position: old,
            });
        }
        Ok(Applied::new(inverse, events))
    }

    fn label(&self) -> String {
        if self.width.is_some() || self.height.is_some() {
            "Resize visual".to_owned()
        } else {
            "Move visual".to_owned()
        }
    }
}
