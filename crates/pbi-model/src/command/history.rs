//! Undo/redo stacks with save-point tracking.

use super::{ChangeEvent, Command, CommandError};
use crate::document::Document;

/// Result of a history operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryResult {
    /// Whether the document content changed.
    pub changed: bool,
    /// Current monotonic document revision.
    pub revision: u64,
    /// Objects affected by this operation.
    pub events: Vec<ChangeEvent>,
}

#[derive(Debug)]
struct Entry {
    label: String,
    command: Box<dyn Command>,
    previous_state: u64,
    next_state: u64,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Undo,
    Redo,
}

/// Undo/redo history for one [`Document`].
///
/// Each state-changing operation advances a monotonic revision. Undo and redo
/// restore content-state identities, so returning to a saved state clears the
/// dirty flag without rewinding the revision.
#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    revision: u64,
    next_state: u64,
    current_state: u64,
    saved_state: Option<u64>,
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
            revision: 0,
            next_state: 1,
            current_state: 0,
            saved_state: Some(0),
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
        self.execute_with_status(doc, command)
            .map(|result| result.events)
    }

    /// Applies a command and returns its change status, revision and events.
    pub fn execute_with_status(
        &mut self,
        doc: &mut Document,
        command: impl Command + 'static,
    ) -> Result<HistoryResult, CommandError> {
        self.execute_boxed_with_status(doc, Box::new(command))
    }

    /// [`History::execute`] for an already boxed command.
    pub fn execute_boxed(
        &mut self,
        doc: &mut Document,
        command: Box<dyn Command>,
    ) -> Result<Vec<ChangeEvent>, CommandError> {
        self.execute_boxed_with_status(doc, command)
            .map(|result| result.events)
    }

    /// [`History::execute_with_status`] for an already boxed command.
    pub fn execute_boxed_with_status(
        &mut self,
        doc: &mut Document,
        command: Box<dyn Command>,
    ) -> Result<HistoryResult, CommandError> {
        self.execute_boxed_checked(doc, command, |_| Ok(()))
            .map(|(result, _)| result)
    }

    /// Like [`History::execute_boxed_with_status`], but runs `check` on the
    /// edited document before committing it. When `check` fails, the
    /// document and the history are unchanged and its error is returned.
    /// `check` runs only when the command changed the document; its value is
    /// returned with the result.
    pub fn execute_boxed_checked<T>(
        &mut self,
        doc: &mut Document,
        command: Box<dyn Command>,
        check: impl FnOnce(&Document) -> Result<T, CommandError>,
    ) -> Result<(HistoryResult, Option<T>), CommandError> {
        let mut working = doc.clone();
        let applied = command.apply(&mut working)?;
        if *doc == working {
            return Ok((self.unchanged(), None));
        }
        let checked = check(&working)?;

        let previous_state = self.current_state;
        let next_state = self.next_state;
        self.next_state = self.next_state.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        *doc = working;
        self.undo.push(Entry {
            label: command.label(),
            command: applied.inverse,
            previous_state,
            next_state,
        });
        self.current_state = next_state;
        self.redo.clear();
        self.trim();
        Ok((
            HistoryResult {
                changed: true,
                revision: self.revision,
                events: applied.events,
            },
            Some(checked),
        ))
    }

    /// Reverts the most recent command.
    pub fn undo(&mut self, doc: &mut Document) -> Result<Vec<ChangeEvent>, CommandError> {
        self.undo_with_status(doc).map(|result| result.events)
    }

    /// Reverts the most recent command and returns its change status, revision
    /// and events.
    pub fn undo_with_status(&mut self, doc: &mut Document) -> Result<HistoryResult, CommandError> {
        self.undo_checked(doc, |_| Ok(())).map(|(result, _)| result)
    }

    /// [`History::undo_with_status`] with a `check` that can refuse the
    /// result (see [`History::execute_boxed_checked`]).
    pub fn undo_checked<T>(
        &mut self,
        doc: &mut Document,
        check: impl FnOnce(&Document) -> Result<T, CommandError>,
    ) -> Result<(HistoryResult, Option<T>), CommandError> {
        self.step(doc, Direction::Undo, check)
    }

    /// Re-applies the most recently undone command.
    pub fn redo(&mut self, doc: &mut Document) -> Result<Vec<ChangeEvent>, CommandError> {
        self.redo_with_status(doc).map(|result| result.events)
    }

    /// Re-applies the most recently undone command and returns its change
    /// status, revision and events.
    pub fn redo_with_status(&mut self, doc: &mut Document) -> Result<HistoryResult, CommandError> {
        self.redo_checked(doc, |_| Ok(())).map(|(result, _)| result)
    }

    /// [`History::redo_with_status`] with a `check` that can refuse the
    /// result (see [`History::execute_boxed_checked`]).
    pub fn redo_checked<T>(
        &mut self,
        doc: &mut Document,
        check: impl FnOnce(&Document) -> Result<T, CommandError>,
    ) -> Result<(HistoryResult, Option<T>), CommandError> {
        self.step(doc, Direction::Redo, check)
    }

    fn unchanged(&self) -> HistoryResult {
        HistoryResult {
            changed: false,
            revision: self.revision,
            events: Vec::new(),
        }
    }

    fn step<T>(
        &mut self,
        doc: &mut Document,
        direction: Direction,
        check: impl FnOnce(&Document) -> Result<T, CommandError>,
    ) -> Result<(HistoryResult, Option<T>), CommandError> {
        let popped = match direction {
            Direction::Undo => self.undo.pop().ok_or(CommandError::NothingToUndo)?,
            Direction::Redo => self.redo.pop().ok_or(CommandError::NothingToRedo)?,
        };
        let mut working = doc.clone();
        let outcome = popped.command.apply(&mut working).and_then(|applied| {
            if *doc == working {
                Ok(None)
            } else {
                check(&working).map(|checked| Some((applied, checked)))
            }
        });
        let (applied, checked) = match outcome {
            Ok(Some(done)) => done,
            Ok(None) => {
                self.stack(direction).push(popped);
                return Ok((self.unchanged(), None));
            }
            Err(err) => {
                self.stack(direction).push(popped);
                return Err(err);
            }
        };
        *doc = working;
        self.revision = self.revision.saturating_add(1);
        self.current_state = match direction {
            Direction::Undo => popped.previous_state,
            Direction::Redo => popped.next_state,
        };
        let moved = Entry {
            label: popped.label,
            command: applied.inverse,
            previous_state: popped.previous_state,
            next_state: popped.next_state,
        };
        match direction {
            Direction::Undo => self.redo.push(moved),
            Direction::Redo => self.undo.push(moved),
        }
        Ok((
            HistoryResult {
                changed: true,
                revision: self.revision,
                events: applied.events,
            },
            Some(checked),
        ))
    }

    fn stack(&mut self, direction: Direction) -> &mut Vec<Entry> {
        match direction {
            Direction::Undo => &mut self.undo,
            Direction::Redo => &mut self.redo,
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

    /// Monotonic document revision, advanced by every changed edit, undo or
    /// redo. No-op operations do not advance it.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// `true` if the document differs from the last saved state.
    pub fn is_dirty(&self) -> bool {
        self.saved_state != Some(self.current_state)
    }

    /// Records the current state as saved.
    pub fn mark_saved(&mut self) {
        self.saved_state = Some(self.current_state);
    }

    /// Drops all undo/redo steps; the dirty flag is preserved.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn trim(&mut self) {
        let Some(limit) = self.limit else { return };
        if self.undo.len() > limit {
            let excess = self.undo.len() - limit;
            self.undo.drain(..excess);
        }
    }
}
