//! Serialized identifiers of new model objects (`lineageTag`, ADR-0014).
//!
//! A session draws them from an injectable [`IdentifierGenerator`]: random
//! by default, a fixed sequence in tests ([`SequentialIdentifiers`]).

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use pbi_model::Model;

/// Source of new serialized identifiers.
pub trait IdentifierGenerator: Send + Sync + fmt::Debug {
    /// A new lineage tag: a lowercase, hyphenated GUID.
    fn lineage_tag(&self) -> String;
}

/// Random version 4 GUIDs from the operating system's generator: the
/// default, so tags are unpredictable and do not collide across projects.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomIdentifiers;

impl IdentifierGenerator for RandomIdentifiers {
    fn lineage_tag(&self) -> String {
        uuid::Uuid::new_v4().hyphenated().to_string()
    }
}

/// A fixed sequence for tests: `{seed:08x}-0000-4000-8000-000000000001`,
/// `…-000000000002`, …
#[derive(Debug, Default)]
pub struct SequentialIdentifiers {
    seed: u32,
    next: AtomicU64,
}

impl SequentialIdentifiers {
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            next: AtomicU64::new(0),
        }
    }
}

impl IdentifierGenerator for SequentialIdentifiers {
    fn lineage_tag(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & 0xffff_ffff_ffff;
        format!("{:08x}-0000-4000-8000-{n:012x}", self.seed)
    }
}

/// Every lineage tag in the model (tables, columns, measures, hierarchies,
/// levels, named expressions).
pub(crate) fn lineage_tags(model: &Model) -> HashSet<&str> {
    let mut tags = HashSet::new();
    for table in &model.tables {
        tags.extend(table.lineage_tag.as_deref());
        tags.extend(
            table
                .columns
                .iter()
                .filter_map(|c| c.lineage_tag.as_deref()),
        );
        tags.extend(
            table
                .measures
                .iter()
                .filter_map(|m| m.lineage_tag.as_deref()),
        );
        for hierarchy in &table.hierarchies {
            tags.extend(hierarchy.lineage_tag.as_deref());
            tags.extend(
                hierarchy
                    .levels
                    .iter()
                    .filter_map(|l| l.lineage_tag.as_deref()),
            );
        }
    }
    tags.extend(
        model
            .expressions
            .iter()
            .filter_map(|e| e.lineage_tag.as_deref()),
    );
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_tags_are_version_4_guids() {
        let a = RandomIdentifiers.lineage_tag();
        let b = RandomIdentifiers.lineage_tag();
        assert_ne!(a, b);
        let parsed = uuid::Uuid::parse_str(&a).expect("a GUID");
        assert_eq!(parsed.get_version_num(), 4);
        assert_eq!(a, a.to_lowercase());
        assert_eq!(a.len(), 36);
    }

    #[test]
    fn sequential_tags_count_up_from_one() {
        let ids = SequentialIdentifiers::new(0xacce);
        assert_eq!(ids.lineage_tag(), "0000acce-0000-4000-8000-000000000001");
        assert_eq!(ids.lineage_tag(), "0000acce-0000-4000-8000-000000000002");
    }
}
