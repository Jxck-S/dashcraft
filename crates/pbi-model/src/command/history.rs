//! Undo/redo stacks with save-point tracking.

use super::{ChangeEvent, Command, CommandError};
use crate::document::Document;

#[derive(Debug)]
struct Entry {
    label: String,
    command: Box<dyn Command>,
    revision: u64,
}

/// Undo/redo history for one [`Document`].
///
/// Each executed command gets a unique revision number; the document is
/// "dirty" when the revision on top of the undo stack differs from the one
/// recorded by [`History::mark_saved`].
#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_revision: u64,
    /// Revision of the state below the oldest undo entry.
    base_revision: u64,
    saved_revision: Option<u64>,
    limit: Option<usize>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// Empty, clean history without a depth limit.
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_revision: 1,
            base_revision: 0,
            saved_revision: Some(0),
            limit: None,
        }
    }

    /// History that keeps at most `limit` undo steps.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            ..Self::new()
        }
    }

    /// Applies `command` and records its inverse. Clears the redo stack.
    pub fn execute(
        &mut self,
        doc: &mut Document,
        command: impl Command + 'static,
    ) -> Result<Vec<ChangeEvent>, CommandError> {
        self.execute_boxed(doc, Box::new(command))
    }

    /// [`History::execute`] for an already boxed command.
    pub fn execute_boxed(
        &mut self,
        doc: &mut Document,
        command: Box<dyn Command>,
    ) -> Result<Vec<ChangeEvent>, CommandError> {
        let applied = command.apply(doc)?;
        let revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        self.undo.push(Entry {
            label: command.label(),
            command: applied.inverse,
            revision,
        });
        self.redo.clear();
        self.trim();
        Ok(applied.events)
    }

    /// Reverts the most recent command.
    pub fn undo(&mut self, doc: &mut Document) -> Result<Vec<ChangeEvent>, CommandError> {
        let entry = self.undo.pop().ok_or(CommandError::NothingToUndo)?;
        match entry.command.apply(doc) {
            Ok(applied) => {
                self.redo.push(Entry {
                    label: entry.label,
                    command: applied.inverse,
                    revision: entry.revision,
                });
                Ok(applied.events)
            }
            Err(err) => {
                self.undo.push(entry);
                Err(err)
            }
        }
    }

    /// Re-applies the most recently undone command.
    pub fn redo(&mut self, doc: &mut Document) -> Result<Vec<ChangeEvent>, CommandError> {
        let entry = self.redo.pop().ok_or(CommandError::NothingToRedo)?;
        match entry.command.apply(doc) {
            Ok(applied) => {
                self.undo.push(Entry {
                    label: entry.label,
                    command: applied.inverse,
                    revision: entry.revision,
                });
                Ok(applied.events)
            }
            Err(err) => {
                self.redo.push(entry);
                Err(err)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label of the command [`History::undo`] would revert.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    /// Label of the command [`History::redo`] would re-apply.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    fn current_revision(&self) -> u64 {
        self.undo.last().map_or(self.base_revision, |e| e.revision)
    }

    /// `true` if the document differs from the last saved state.
    pub fn is_dirty(&self) -> bool {
        self.saved_revision != Some(self.current_revision())
    }

    /// Records the current state as saved.
    pub fn mark_saved(&mut self) {
        self.saved_revision = Some(self.current_revision());
    }

    /// Drops all undo/redo steps; the dirty flag is preserved.
    pub fn clear(&mut self) {
        self.base_revision = self.current_revision();
        self.undo.clear();
        self.redo.clear();
    }

    fn trim(&mut self) {
        let Some(limit) = self.limit else { return };
        if self.undo.len() > limit {
            let excess = self.undo.len() - limit;
            if let Some(last_dropped) = self.undo.drain(..excess).next_back() {
                self.base_revision = last_dropped.revision;
            }
        }
    }
}
