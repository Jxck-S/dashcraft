//! Desktop-ready command layer: every edit is a [`Command`] whose `apply`
//! returns its exact inverse, so [`History`] can undo and redo any edit.
//!
//! Rules for command authors:
//! - Validate everything before mutating; a failed `apply` must leave the
//!   document untouched.
//! - The inverse must restore the *exact* prior state (tests compare whole
//!   documents with `==`). Structural edits that touch many objects can use
//!   [`ReplaceObjects`], whose inverse swaps the previous values back.
//! - Address objects by [`ObjectId`], never by index or name, so commands
//!   stay valid across renames and reorders.
//! - Report what changed as [`ChangeEvent`]s so a UI can refresh.

mod edit;
mod history;
mod rename;
mod snapshot;

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::semantic::Model;

pub use edit::{SetDescription, SetMeasureExpression, SetVisualPosition};
pub use history::History;
pub use rename::{RenameColumn, RenameMeasure, RenameTable};
pub use snapshot::{ReplaceObjects, Snapshot};

/// A reversible edit.
pub trait Command: fmt::Debug + Send {
    /// Applies the edit and returns its inverse plus change events.
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError>;

    /// Short human-readable description for undo/redo menus.
    fn label(&self) -> String;
}

/// Result of [`Command::apply`].
#[derive(Debug)]
pub struct Applied {
    /// Command that reverts the edit exactly.
    pub inverse: Box<dyn Command>,
    /// What changed, for UI refresh.
    pub events: Vec<ChangeEvent>,
}

impl Applied {
    pub fn new(inverse: impl Command + 'static, events: Vec<ChangeEvent>) -> Self {
        Self {
            inverse: Box::new(inverse),
            events,
        }
    }
}

/// Notification about a changed object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChangeEvent {
    ObjectChanged {
        id: ObjectId,
    },
    ObjectRenamed {
        id: ObjectId,
        old: String,
        new: String,
    },
    ObjectAdded {
        id: ObjectId,
    },
    ObjectRemoved {
        id: ObjectId,
    },
}

impl ChangeEvent {
    pub fn id(&self) -> ObjectId {
        match self {
            ChangeEvent::ObjectChanged { id }
            | ChangeEvent::ObjectRenamed { id, .. }
            | ChangeEvent::ObjectAdded { id }
            | ChangeEvent::ObjectRemoved { id } => *id,
        }
    }
}

/// Why a command could not be applied. The document is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("object {0} not found")]
    NotFound(ObjectId),
    #[error("object {id} is a {actual:?}, expected a {expected:?}")]
    WrongKind {
        id: ObjectId,
        expected: ObjectKind,
        actual: ObjectKind,
    },
    #[error("the document has no semantic model")]
    NoModel,
    #[error("invalid name {0:?}")]
    InvalidName(String),
    #[error("the name {name:?} is already used by {existing}")]
    NameConflict { name: String, existing: String },
    #[error("{kind:?} objects do not support {operation}")]
    Unsupported {
        kind: ObjectKind,
        operation: &'static str,
    },
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
}

/// Several commands applied as one undo step. If one fails, the ones
/// already applied are reverted.
#[derive(Debug)]
pub struct CommandGroup {
    pub label: String,
    pub commands: Vec<Box<dyn Command>>,
}

impl CommandGroup {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            commands: Vec::new(),
        }
    }

    pub fn with(mut self, command: impl Command + 'static) -> Self {
        self.commands.push(Box::new(command));
        self
    }
}

impl Command for CommandGroup {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let mut inverses: Vec<Box<dyn Command>> = Vec::with_capacity(self.commands.len());
        let mut events = Vec::new();
        for command in &self.commands {
            match command.apply(doc) {
                Ok(applied) => {
                    inverses.push(applied.inverse);
                    events.extend(applied.events);
                }
                Err(err) => {
                    for inverse in inverses.iter().rev() {
                        // Inverses of just-applied commands cannot fail on an
                        // unchanged document; nothing better to do if they do.
                        let _ = inverse.apply(doc);
                    }
                    return Err(err);
                }
            }
        }
        inverses.reverse();
        Ok(Applied::new(
            CommandGroup {
                label: self.label.clone(),
                commands: inverses,
            },
            events,
        ))
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// Locates `id` and checks its kind.
pub(crate) fn expect_kind(
    doc: &Document,
    id: ObjectId,
    expected: ObjectKind,
) -> Result<ObjectPath, CommandError> {
    let path = doc.locate(id).ok_or(CommandError::NotFound(id))?;
    let actual = path.kind();
    if actual == expected {
        Ok(path)
    } else {
        Err(CommandError::WrongKind {
            id,
            expected,
            actual,
        })
    }
}

pub(crate) fn model(doc: &Document) -> Result<&Model, CommandError> {
    doc.model.as_ref().ok_or(CommandError::NoModel)
}

pub(crate) fn model_mut(doc: &mut Document) -> Result<&mut Model, CommandError> {
    doc.model.as_mut().ok_or(CommandError::NoModel)
}

#[cfg(test)]
mod tests;
