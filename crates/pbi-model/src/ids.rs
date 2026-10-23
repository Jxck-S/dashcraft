//! Stable object identities.
//!
//! Every model and report object carries an [`ObjectId`]. IDs are assigned
//! when a document is loaded ([`crate::Document::assign_ids`]) and never
//! change afterwards, even when the object is renamed. That gives undo/redo
//! and UI selection a handle that survives renames.
//!
//! `ObjectId(0)` means "not assigned yet". Readers can build objects with
//! default IDs and let the document assign real ones.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Opaque, document-unique object identity. `0` means unassigned.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ObjectId(pub u64);

impl ObjectId {
    /// The "not assigned yet" sentinel.
    pub const UNASSIGNED: ObjectId = ObjectId(0);

    /// `true` once a real ID has been assigned.
    pub fn is_assigned(self) -> bool {
        self.0 != 0
    }

    /// Raw numeric value.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Monotonic [`ObjectId`] generator owned by a [`crate::Document`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IdGen {
    next: u64,
}

impl Default for IdGen {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl IdGen {
    /// A generator whose first ID is `#1`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a fresh, never-before-issued ID.
    pub fn next_id(&mut self) -> ObjectId {
        let id = ObjectId(self.next.max(1));
        self.next = id.0.saturating_add(1);
        id
    }

    /// The ID the next call to [`IdGen::next_id`] will return.
    pub fn peek(&self) -> ObjectId {
        ObjectId(self.next.max(1))
    }

    /// Makes sure future IDs are greater than `id` (used after loading
    /// objects that already carry IDs, e.g. from a saved session).
    pub fn reserve_through(&mut self, id: ObjectId) {
        if self.next <= id.0 {
            self.next = id.0.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_respect_reservations() {
        let mut ids = IdGen::new();
        let a = ids.next_id();
        let b = ids.next_id();
        assert_ne!(a, b);
        assert!(a.is_assigned() && b.is_assigned());
        ids.reserve_through(ObjectId(41));
        assert_eq!(ids.next_id(), ObjectId(42));
        // Reserving below the current position never moves backwards.
        ids.reserve_through(ObjectId(3));
        assert_eq!(ids.next_id(), ObjectId(43));
    }
}
