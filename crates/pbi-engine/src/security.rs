//! Row-level security (RLS) and the effective user of an engine.
//!
//! Power BI applies a role's table filters as *security filters*: they are
//! not part of any filter context (`ALL` cannot remove them), every query
//! simply sees fewer rows. The storage engine reproduces that by building a
//! restricted copy of the data ([`crate::Engine::with_row_security`]):
//!
//! 1. each role's [`RoleFilter`]s (the rows of a table its DAX filter
//!    expression accepts, computed by the formula engine) restrict their
//!    tables and propagate along the active relationships in their
//!    *security* direction ([`RelationshipGraph::for_security`]:
//!    `securityFilteringBehavior` `oneDirection` → one → many only,
//!    `bothDirections` → both ways, `none` → not crossed);
//! 2. the rows a user may see are the **union over the roles** of the rows
//!    each role allows ([`EngineSnapshot::security_masks`]);
//! 3. every table keeps only those rows, and relationship indexes and blank
//!    rows are rebuilt on the restricted data.
//!
//! The effective user ([`UserIdentity`], for `USERPRINCIPALNAME()`,
//! `USERNAME()`, `USEROBJECTID()` and `CUSTOMDATA()`) travels with the
//! snapshot ([`EngineSnapshot::security`]).

use std::collections::HashMap;

use pbi_model::names::name_key;

use crate::engine::EngineSnapshot;
use crate::error::Result;
use crate::filter::{FilterContext, TableFilter, VisibleRows};
use crate::relationships::RelationshipGraph;
use crate::rowset::RowSet;

/// The effective user of an evaluation ("View as" → *Other user*).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct UserIdentity {
    /// `USERPRINCIPALNAME()` and `USERNAME()` (e.g. `user@contoso.com`).
    pub user_name: Option<String>,
    /// `USEROBJECTID()`.
    pub user_object_id: Option<String>,
    /// `CUSTOMDATA()`.
    pub custom_data: Option<String>,
}

/// Security settings carried by an engine snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityContext {
    /// Roles whose filters restrict the data (empty: no row-level
    /// security).
    pub roles: Vec<String>,
    /// The effective user.
    pub identity: UserIdentity,
}

/// The rows of one table a role's filter expression accepts.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleFilter {
    pub table: String,
    /// Accepted rows (universe: the table's row count).
    pub rows: RowSet,
    /// The table's blank row (all columns BLANK) is accepted.
    pub blank_row: bool,
}

/// One role's table filters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SecurityRole {
    pub name: String,
    pub filters: Vec<RoleFilter>,
}

impl EngineSnapshot {
    /// The rows of every stored table (by case-folded name) a user in
    /// `roles` may see: per role, its filters restrict their tables and
    /// propagate on [`RelationshipGraph::for_security`]; the result is the
    /// union over the roles. No roles: no rows.
    pub fn security_masks(&self, roles: &[SecurityRole]) -> Result<HashMap<String, VisibleRows>> {
        let storage = self.storage();
        let graph: RelationshipGraph = self.graph().for_security(storage);
        let mut out: HashMap<String, VisibleRows> = storage
            .tables()
            .map(|t| {
                (
                    name_key(t.name()),
                    VisibleRows {
                        rows: RowSet::empty(t.row_count()),
                        blank_row: false,
                    },
                )
            })
            .collect();
        for role in roles {
            let mut ctx = FilterContext::new();
            for f in &role.filters {
                let data = storage.table(&f.table)?;
                ctx.add(
                    TableFilter::new(data.name(), f.rows.resized(data.row_count()))
                        .base_only()
                        .with_blank_row(f.blank_row),
                );
            }
            for table in storage.tables() {
                let visible = graph.visible_on_graph(storage, &ctx, table.name())?;
                if let Some(acc) = out.get_mut(&name_key(table.name())) {
                    acc.rows.union_with(&visible.rows);
                    acc.blank_row |= visible.blank_row;
                }
            }
        }
        Ok(out)
    }
}
