//! Commands that add and remove objects.
//!
//! New objects get a fresh [`ObjectId`] from the document's generator. Undo
//! removes them again but never rewinds the generator, so an ID handed to a
//! caller is never reused for another object. Serialized identifiers
//! (`lineageTag`) come with the object: the session draws them before the
//! command is built (ADR-0014), so undo and redo keep them.

use super::edit::{changed, swap};
use super::rename::{check_measure_name, validate_name};
use super::{Applied, ChangeEvent, Command, CommandError, expect_kind, model, model_mut};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::names::{dax_column_ref, names_eq};
use crate::semantic::{
    Cardinality, Column, ColumnKind, ColumnRef, CrossFilter, Measure, Model, Relationship,
    SecurityFilteringBehavior, Table, TablePermission,
};

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

/// Adds a relationship. Its `id` is ignored: it gets a fresh one (see
/// [`CreateRelationship::created_id`]).
///
/// Checks: a valid name not used by another relationship, both columns
/// exist on two different tables, no relationship already joins the same
/// two columns, and an active one is the only active relationship between
/// its two tables. Desktop's further checks (ambiguous paths, key
/// uniqueness in the data) are left to the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateRelationship {
    pub relationship: Relationship,
    /// Position among the model's relationships; `None` (or past the end)
    /// appends.
    pub index: Option<usize>,
}

impl CreateRelationship {
    /// The ID the relationship gets when this command is applied to `doc`
    /// now.
    pub fn created_id(doc: &Document) -> ObjectId {
        doc.ids.peek()
    }
}

impl Command for CreateRelationship {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        check_relationship(model(doc)?, &self.relationship)?;
        let mut relationship = self.relationship.clone();
        relationship.id = doc.ids.peek();
        let id = relationship.id;
        insert_relationship(doc, relationship, self.index)?;
        doc.ids.reserve_through(id);
        Ok(Applied::new(
            DeleteRelationship { relationship: id },
            vec![ChangeEvent::ObjectAdded { id }],
        ))
    }

    fn label(&self) -> String {
        format!(
            "Create relationship {} → {}",
            dax_column_ref(
                &self.relationship.from.table,
                &self.relationship.from.column
            ),
            dax_column_ref(&self.relationship.to.table, &self.relationship.to.column)
        )
    }
}

/// Removes a relationship. DAX that names it through `USERELATIONSHIP` or
/// `CROSSFILTER` is left as it is; undo puts it back where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRelationship {
    pub relationship: ObjectId,
}

impl Command for DeleteRelationship {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Relationship { relationship: i } =
            expect_kind(doc, self.relationship, ObjectKind::Relationship)?
        else {
            return Err(CommandError::NotFound(self.relationship));
        };
        let relationships = &mut model_mut(doc)?.relationships;
        if i >= relationships.len() {
            return Err(CommandError::NotFound(self.relationship));
        }
        let relationship = relationships.remove(i);
        Ok(Applied::new(
            RestoreRelationship {
                relationship,
                index: i,
            },
            vec![ChangeEvent::ObjectRemoved {
                id: self.relationship,
            }],
        ))
    }

    fn label(&self) -> String {
        "Delete relationship".to_owned()
    }
}

/// Inverse of [`DeleteRelationship`]: puts the relationship back as it was,
/// ID included, without re-validating it.
#[derive(Debug, Clone, PartialEq)]
struct RestoreRelationship {
    relationship: Relationship,
    index: usize,
}

impl Command for RestoreRelationship {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let id = self.relationship.id;
        if !id.is_assigned() || doc.locate(id).is_some() {
            return Err(CommandError::Rejected(format!(
                "cannot restore relationship '{}': its ID {id} is in use",
                self.relationship.name
            )));
        }
        insert_relationship(doc, self.relationship.clone(), Some(self.index))?;
        doc.ids.reserve_through(id);
        Ok(Applied::new(
            DeleteRelationship { relationship: id },
            vec![ChangeEvent::ObjectAdded { id }],
        ))
    }

    fn label(&self) -> String {
        "Restore relationship".to_owned()
    }
}

/// Patches a relationship's properties in one step; `None` leaves a
/// property unchanged. Its name and columns stay: to join other columns,
/// delete it and create another.
///
/// Checks: a relationship made active is the only active one between its
/// two tables (deactivate the other first, in the same group for one undo
/// step), and security filters cross both ways only when filters do
/// (`securityFilteringBehavior: bothDirections` needs
/// `crossFilteringBehavior: bothDirections`). Undo and redo restore the
/// previous state without checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetRelationshipProperties {
    pub relationship: ObjectId,
    pub from_cardinality: Option<Cardinality>,
    pub to_cardinality: Option<Cardinality>,
    pub cross_filtering_behavior: Option<CrossFilter>,
    pub security_filtering_behavior: Option<SecurityFilteringBehavior>,
    pub is_active: Option<bool>,
}

impl SetRelationshipProperties {
    fn patch(&self, doc: &mut Document, check: bool) -> Result<Applied, CommandError> {
        let ObjectPath::Relationship { relationship: i } =
            expect_kind(doc, self.relationship, ObjectKind::Relationship)?
        else {
            return Err(CommandError::NotFound(self.relationship));
        };
        let current = model(doc)?
            .relationships
            .get(i)
            .ok_or(CommandError::NotFound(self.relationship))?;
        let mut next = current.clone();
        let mut differs = false;
        let inverse = SetRelationshipProperties {
            relationship: self.relationship,
            from_cardinality: swap(
                &mut next.from_cardinality,
                &self.from_cardinality,
                &mut differs,
            ),
            to_cardinality: swap(&mut next.to_cardinality, &self.to_cardinality, &mut differs),
            cross_filtering_behavior: swap(
                &mut next.cross_filtering_behavior,
                &self.cross_filtering_behavior,
                &mut differs,
            ),
            security_filtering_behavior: swap(
                &mut next.security_filtering_behavior,
                &self.security_filtering_behavior,
                &mut differs,
            ),
            is_active: swap(&mut next.is_active, &self.is_active, &mut differs),
        };
        if check {
            if next.is_active && !current.is_active {
                check_only_active(
                    model(doc)?,
                    &next,
                    Some(self.relationship),
                    "make that one inactive first",
                )?;
            }
            let filters_edited = self.cross_filtering_behavior.is_some()
                || self.security_filtering_behavior.is_some();
            if filters_edited
                && next.security_filtering_behavior == SecurityFilteringBehavior::BothDirections
                && next.cross_filtering_behavior != CrossFilter::BothDirections
            {
                return Err(CommandError::Rejected(format!(
                    "relationship {}: security filters cross both ways only when filters do; set crossFilteringBehavior to bothDirections too",
                    label(&next)
                )));
            }
        }
        let slot = model_mut(doc)?
            .relationships
            .get_mut(i)
            .ok_or(CommandError::NotFound(self.relationship))?;
        *slot = next;
        Ok(Applied::new(
            RestoreRelationshipProperties(inverse),
            changed(self.relationship, differs),
        ))
    }
}

impl Command for SetRelationshipProperties {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        self.patch(doc, true)
    }

    fn label(&self) -> String {
        "Edit relationship".to_owned()
    }
}

/// Inverse of [`SetRelationshipProperties`]: puts back properties that were
/// accepted before, without the checks (a model may come with two active
/// relationships between the same tables; undo restores that as it was).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreRelationshipProperties(SetRelationshipProperties);

impl Command for RestoreRelationshipProperties {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        self.0.patch(doc, false)
    }

    fn label(&self) -> String {
        "Edit relationship".to_owned()
    }
}

fn insert_relationship(
    doc: &mut Document,
    relationship: Relationship,
    index: Option<usize>,
) -> Result<(), CommandError> {
    let relationships = &mut model_mut(doc)?.relationships;
    let index = index.map_or(relationships.len(), |i| i.min(relationships.len()));
    relationships.insert(index, relationship);
    Ok(())
}

/// The checks of [`CreateRelationship`].
fn check_relationship(model: &Model, new: &Relationship) -> Result<(), CommandError> {
    validate_name(&new.name)?;
    if let Some(other) = model
        .relationships
        .iter()
        .find(|r| names_eq(&r.name, &new.name))
    {
        return Err(CommandError::NameConflict {
            name: new.name.clone(),
            existing: format!("relationship {}", label(other)),
        });
    }
    for end in [&new.from, &new.to] {
        if model.column(&end.table, &end.column).is_none() {
            return Err(CommandError::Rejected(format!(
                "there is no column {}",
                dax_column_ref(&end.table, &end.column)
            )));
        }
    }
    if names_eq(&new.from.table, &new.to.table) {
        return Err(CommandError::Rejected(format!(
            "a relationship joins two different tables; both ends are on '{}'",
            new.from.table
        )));
    }
    let same_ends = |a: &ColumnRef, b: &ColumnRef| a.matches(&b.table, &b.column);
    for r in &model.relationships {
        if (same_ends(&r.from, &new.from) && same_ends(&r.to, &new.to))
            || (same_ends(&r.from, &new.to) && same_ends(&r.to, &new.from))
        {
            return Err(CommandError::Rejected(format!(
                "relationship {} already joins these columns",
                label(r)
            )));
        }
    }
    if new.is_active {
        check_only_active(model, new, None, "create this one inactive")?;
    }
    Ok(())
}

/// Rejects `r` (assumed active) when another active relationship, other
/// than `skip`, joins the same two tables.
fn check_only_active(
    model: &Model,
    r: &Relationship,
    skip: Option<ObjectId>,
    hint: &str,
) -> Result<(), CommandError> {
    let same_tables = |o: &Relationship| {
        (names_eq(&o.from.table, &r.from.table) && names_eq(&o.to.table, &r.to.table))
            || (names_eq(&o.from.table, &r.to.table) && names_eq(&o.to.table, &r.from.table))
    };
    match model
        .relationships
        .iter()
        .find(|o| Some(o.id) != skip && o.is_active && same_tables(o))
    {
        Some(other) => Err(CommandError::Rejected(format!(
            "'{}' and '{}' already have an active relationship ({}); {hint}",
            r.from.table,
            r.to.table,
            label(other)
        ))),
        None => Ok(()),
    }
}

fn label(r: &Relationship) -> String {
    format!(
        "{} → {}",
        dax_column_ref(&r.from.table, &r.from.column),
        dax_column_ref(&r.to.table, &r.to.column)
    )
}

/// Removes a calculated column. Data columns (they come from the table's
/// query) and calculated-table columns (they come from the table's
/// expression) are refused, and so is a column that a relationship, a
/// hierarchy level or another column's sort order uses: delete or change
/// those first (in the same group for one undo step). DAX and report fields
/// that refer to the column are left as they are; undo puts the column back
/// where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteColumn {
    pub column: ObjectId,
}

impl Command for DeleteColumn {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Column {
            table: t,
            column: c,
        } = expect_kind(doc, self.column, ObjectKind::Column)?
        else {
            return Err(CommandError::NotFound(self.column));
        };
        let model = model(doc)?;
        let table = model
            .tables
            .get(t)
            .ok_or(CommandError::NotFound(self.column))?;
        let column = table
            .columns
            .get(c)
            .ok_or(CommandError::NotFound(self.column))?;
        let what = dax_column_ref(&table.name, &column.name);
        let refusal = match &column.kind {
            ColumnKind::Calculated { .. } => None,
            ColumnKind::Data { .. } => Some(
                "is a data column: it comes from the table's query, which cannot be edited yet; only calculated columns can be deleted",
            ),
            ColumnKind::CalculatedTableColumn { .. } => {
                Some("comes from the calculated table's expression; change the expression instead")
            }
            ColumnKind::RowNumber => Some("is the table's internal row-number column"),
        };
        if let Some(reason) = refusal {
            return Err(CommandError::Rejected(format!("{what} {reason}")));
        }
        let uses = column_uses(model, table, column);
        if !uses.is_empty() {
            return Err(CommandError::Rejected(format!(
                "{what} is used by {}; delete or change those first",
                uses.join(", ")
            )));
        }
        let table_id = table.id;
        let columns = &mut model_mut(doc)?
            .tables
            .get_mut(t)
            .ok_or(CommandError::NotFound(self.column))?
            .columns;
        if c >= columns.len() {
            return Err(CommandError::NotFound(self.column));
        }
        let column = columns.remove(c);
        Ok(Applied::new(
            RestoreColumn {
                table: table_id,
                column,
                index: c,
            },
            vec![ChangeEvent::ObjectRemoved { id: self.column }],
        ))
    }

    fn label(&self) -> String {
        "Delete column".to_owned()
    }
}

/// What in the model structure uses a column: relationships, hierarchy
/// levels and other columns' sort order.
fn column_uses(model: &Model, table: &Table, column: &Column) -> Vec<String> {
    let mut uses: Vec<String> = model
        .relationships
        .iter()
        .filter(|r| {
            r.from.matches(&table.name, &column.name) || r.to.matches(&table.name, &column.name)
        })
        .map(|r| format!("relationship {}", label(r)))
        .collect();
    for h in &table.hierarchies {
        for l in h
            .levels
            .iter()
            .filter(|l| names_eq(&l.column, &column.name))
        {
            uses.push(format!("level '{}' of hierarchy '{}'", l.name, h.name));
        }
    }
    for other in &table.columns {
        if other.id != column.id
            && other
                .sort_by_column
                .as_deref()
                .is_some_and(|s| names_eq(s, &column.name))
        {
            uses.push(format!(
                "the sort order of {}",
                dax_column_ref(&table.name, &other.name)
            ));
        }
    }
    uses
}

/// Inverse of [`DeleteColumn`]: puts the column back as it was, ID
/// included, without re-validating it.
#[derive(Debug, Clone, PartialEq)]
struct RestoreColumn {
    table: ObjectId,
    column: Column,
    index: usize,
}

impl Command for RestoreColumn {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let id = self.column.id;
        if !id.is_assigned() || doc.locate(id).is_some() {
            return Err(CommandError::Rejected(format!(
                "cannot restore column '{}': its ID {id} is in use",
                self.column.name
            )));
        }
        let ObjectPath::Table { table: t } = expect_kind(doc, self.table, ObjectKind::Table)?
        else {
            return Err(CommandError::NotFound(self.table));
        };
        let columns = &mut model_mut(doc)?
            .tables
            .get_mut(t)
            .ok_or(CommandError::NotFound(self.table))?
            .columns;
        columns.insert(self.index.min(columns.len()), self.column.clone());
        doc.ids.reserve_through(id);
        Ok(Applied::new(
            DeleteColumn { column: id },
            vec![ChangeEvent::ObjectAdded { id }],
        ))
    }

    fn label(&self) -> String {
        format!("Restore column '{}'", self.column.name)
    }
}

/// Removes a table with its columns, measures and hierarchies, the
/// relationships that join it and the roles' row filters on it, as Power BI
/// Desktop does. DAX and report fields that refer to the table are left as
/// they are; undo puts everything back where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTable {
    pub table: ObjectId,
}

impl Command for DeleteTable {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let ObjectPath::Table { table: t } = expect_kind(doc, self.table, ObjectKind::Table)?
        else {
            return Err(CommandError::NotFound(self.table));
        };
        let model = model_mut(doc)?;
        if t >= model.tables.len() {
            return Err(CommandError::NotFound(self.table));
        }
        let name = model
            .tables
            .get(t)
            .map(|x| x.name.clone())
            .unwrap_or_default();
        let mut events = vec![ChangeEvent::ObjectRemoved { id: self.table }];
        let mut relationships = Vec::new();
        let mut index = 0;
        model.relationships.retain(|r| {
            let joins = names_eq(&r.from.table, &name) || names_eq(&r.to.table, &name);
            if joins {
                relationships.push((index, r.clone()));
                events.push(ChangeEvent::ObjectRemoved { id: r.id });
            }
            index += 1;
            !joins
        });
        let mut permissions = Vec::new();
        for role in &mut model.roles {
            let mut index = 0;
            role.table_permissions.retain(|p| {
                let on_table = names_eq(&p.table, &name);
                if on_table {
                    permissions.push((role.id, index, p.clone()));
                    events.push(ChangeEvent::ObjectRemoved { id: p.id });
                }
                index += 1;
                !on_table
            });
        }
        let table = model.tables.remove(t);
        Ok(Applied::new(
            RestoreTable {
                table,
                index: t,
                relationships,
                permissions,
            },
            events,
        ))
    }

    fn label(&self) -> String {
        "Delete table".to_owned()
    }
}

/// Inverse of [`DeleteTable`]: puts the table, its relationships and the
/// roles' row filters on it back as they were, IDs included.
#[derive(Debug, Clone, PartialEq)]
struct RestoreTable {
    table: Table,
    index: usize,
    /// Relationships with their positions, in ascending order.
    relationships: Vec<(usize, Relationship)>,
    /// Row filters with their role and position, in ascending order per role.
    permissions: Vec<(ObjectId, usize, TablePermission)>,
}

impl Command for RestoreTable {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let id = self.table.id;
        let ids: Vec<ObjectId> = std::iter::once(id)
            .chain(self.relationships.iter().map(|(_, r)| r.id))
            .chain(self.permissions.iter().map(|(_, _, p)| p.id))
            .collect();
        for &used in &ids {
            if !used.is_assigned() || doc.locate(used).is_some() {
                return Err(CommandError::Rejected(format!(
                    "cannot restore table '{}': ID {used} is in use",
                    self.table.name
                )));
            }
        }
        let mut roles = Vec::new();
        for (role, _, _) in &self.permissions {
            let ObjectPath::Role { role: r } = expect_kind(doc, *role, ObjectKind::Role)? else {
                return Err(CommandError::NotFound(*role));
            };
            if !roles.iter().any(|(id, _)| id == role) {
                roles.push((*role, r));
            }
        }
        let model = model_mut(doc)?;
        model
            .tables
            .insert(self.index.min(model.tables.len()), self.table.clone());
        let mut events = vec![ChangeEvent::ObjectAdded { id }];
        for (i, r) in &self.relationships {
            let at = (*i).min(model.relationships.len());
            model.relationships.insert(at, r.clone());
            events.push(ChangeEvent::ObjectAdded { id: r.id });
        }
        for (role, i, p) in &self.permissions {
            let Some(r) = roles
                .iter()
                .find(|(id, _)| id == role)
                .and_then(|(_, r)| model.roles.get_mut(*r))
            else {
                continue;
            };
            let at = (*i).min(r.table_permissions.len());
            r.table_permissions.insert(at, p.clone());
            events.push(ChangeEvent::ObjectAdded { id: p.id });
        }
        for used in ids {
            doc.ids.reserve_through(used);
        }
        Ok(Applied::new(DeleteTable { table: id }, events))
    }

    fn label(&self) -> String {
        format!("Restore table '{}'", self.table.name)
    }
}
