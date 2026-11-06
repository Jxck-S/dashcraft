//! Commands that add and remove objects.
//!
//! New objects get a fresh [`ObjectId`] from the document's generator. Undo
//! removes them again but never rewinds the generator, so an ID handed to a
//! caller is never reused for another object. Serialized identifiers
//! (`lineageTag`) are left to the writers (ADR-0014).

use super::rename::check_measure_name;
use super::{Applied, ChangeEvent, Command, CommandError, expect_kind, model, model_mut};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::semantic::Measure;

/// Adds a measure to a table. The measure's `id` is ignored: it gets a fresh
/// one (see [`Applied::events`] or [`CreateMeasure::created_id`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMeasure {
    pub table: ObjectId,
    pub measure: Measure,
    /// Position among the table's measures; `None` (or past the end) appends.
    pub index: Option<usize>,
}

impl CreateMeasure {
    /// The ID the measure gets when this command is applied to `doc` now.
    pub fn created_id(doc: &Document) -> ObjectId {
        doc.ids.peek()
    }
}

impl Command for CreateMeasure {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Table { table: t } = expect_kind(doc, self.table, ObjectKind::Table)?
        else {
            return Err(CommandError::NotFound(self.table));
        };
        check_measure_name(model(doc)?, t, ObjectId::UNASSIGNED, &self.measure.name)?;
        let mut measure = self.measure.clone();
        measure.id = doc.ids.peek();
        let id = measure.id;
        insert(doc, t, self.table, measure, self.index)?;
        doc.ids.reserve_through(id);
        Ok(Applied::new(
            DeleteMeasure { measure: id },
            vec![ChangeEvent::ObjectAdded { id }],
        ))
    }

    fn label(&self) -> String {
        format!("Create measure '{}'", self.measure.name)
    }
}

/// Removes a measure. References to it (DAX, report fields) are left as they
/// are; undo puts the measure back exactly where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteMeasure {
    pub measure: ObjectId,
}

impl Command for DeleteMeasure {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Measure {
            table: t,
            measure: m,
        } = expect_kind(doc, self.measure, ObjectKind::Measure)?
        else {
            return Err(CommandError::NotFound(self.measure));
        };
        let table = model_mut(doc)?
            .tables
            .get_mut(t)
            .ok_or(CommandError::NotFound(self.measure))?;
        if m >= table.measures.len() {
            return Err(CommandError::NotFound(self.measure));
        }
        let measure = table.measures.remove(m);
        Ok(Applied::new(
            RestoreMeasure {
                table: table.id,
                measure,
                index: m,
            },
            vec![ChangeEvent::ObjectRemoved { id: self.measure }],
        ))
    }

    fn label(&self) -> String {
        "Delete measure".to_owned()
    }
}

/// Inverse of [`DeleteMeasure`]: puts the measure back as it was, ID
/// included, without re-validating its name.
#[derive(Debug, Clone, PartialEq)]
struct RestoreMeasure {
    table: ObjectId,
    measure: Measure,
    index: usize,
}

impl Command for RestoreMeasure {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let id = self.measure.id;
        if !id.is_assigned() || doc.locate(id).is_some() {
            return Err(CommandError::Rejected(format!(
                "cannot restore measure '{}': its ID {id} is in use",
                self.measure.name
            )));
        }
        let ObjectPath::Table { table: t } = expect_kind(doc, self.table, ObjectKind::Table)?
        else {
            return Err(CommandError::NotFound(self.table));
        };
        insert(doc, t, self.table, self.measure.clone(), Some(self.index))?;
        doc.ids.reserve_through(id);
        Ok(Applied::new(
            DeleteMeasure { measure: id },
            vec![ChangeEvent::ObjectAdded { id }],
        ))
    }

    fn label(&self) -> String {
        format!("Restore measure '{}'", self.measure.name)
    }
}

fn insert(
    doc: &mut Document,
    table: usize,
    table_id: ObjectId,
    measure: Measure,
    index: Option<usize>,
) -> Result<(), CommandError> {
    let measures = &mut model_mut(doc)?
        .tables
        .get_mut(table)
        .ok_or(CommandError::NotFound(table_id))?
        .measures;
    let index = index.map_or(measures.len(), |i| i.min(measures.len()));
    measures.insert(index, measure);
    Ok(())
}
