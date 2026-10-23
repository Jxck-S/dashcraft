//! Whole-object replacement: the generic, exactly reversible edit used by
//! multi-object commands such as renames.

use std::mem;

use super::{Applied, ChangeEvent, Command, CommandError, model_mut};
use crate::document::{Document, ObjectKind, ObjectPath};
use crate::ids::ObjectId;
use crate::query::Filter;
use crate::report::Visual;
use crate::semantic::{Column, Measure, NamedExpression, Relationship, Role, Table};

/// New value for one object (or object part), addressed by ID.
#[derive(Debug, Clone, PartialEq)]
pub enum Snapshot {
    Table(Table),
    Column(Column),
    Measure(Measure),
    Relationship(Relationship),
    Expression(NamedExpression),
    Role(Role),
    Visual(Visual),
    /// Report-level filter list of the report with this ID.
    ReportFilters {
        report: ObjectId,
        filters: Vec<Filter>,
    },
    /// Page-level filter list of the page with this ID.
    PageFilters {
        page: ObjectId,
        filters: Vec<Filter>,
    },
}

impl Snapshot {
    /// ID of the object this snapshot replaces.
    pub fn target(&self) -> ObjectId {
        match self {
            Snapshot::Table(x) => x.id,
            Snapshot::Column(x) => x.id,
            Snapshot::Measure(x) => x.id,
            Snapshot::Relationship(x) => x.id,
            Snapshot::Expression(x) => x.id,
            Snapshot::Role(x) => x.id,
            Snapshot::Visual(x) => x.id,
            Snapshot::ReportFilters { report, .. } => *report,
            Snapshot::PageFilters { page, .. } => *page,
        }
    }

    fn target_kind(&self) -> ObjectKind {
        match self {
            Snapshot::Table(_) => ObjectKind::Table,
            Snapshot::Column(_) => ObjectKind::Column,
            Snapshot::Measure(_) => ObjectKind::Measure,
            Snapshot::Relationship(_) => ObjectKind::Relationship,
            Snapshot::Expression(_) => ObjectKind::Expression,
            Snapshot::Role(_) => ObjectKind::Role,
            Snapshot::Visual(_) => ObjectKind::Visual,
            Snapshot::ReportFilters { .. } => ObjectKind::Report,
            Snapshot::PageFilters { .. } => ObjectKind::Page,
        }
    }
}

/// Replaces objects with new values; the inverse puts the old values back.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplaceObjects {
    pub label: String,
    pub items: Vec<Snapshot>,
}

impl Command for ReplaceObjects {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        for item in &self.items {
            super::expect_kind(doc, item.target(), item.target_kind())?;
        }
        let mut previous = Vec::with_capacity(self.items.len());
        let mut events = Vec::new();
        for item in &self.items {
            let mut value = item.clone();
            if let Err(err) = swap_in(doc, &mut value) {
                // Only reachable if items overlap structurally; undo the
                // swaps done so far so the document stays unchanged.
                for old in previous.iter_mut().rev() {
                    let _ = swap_in(doc, old);
                }
                return Err(err);
            }
            diff_events(&value, item, &mut events);
            previous.push(value);
        }
        previous.reverse();
        Ok(Applied::new(
            ReplaceObjects {
                label: self.label.clone(),
                items: previous,
            },
            events,
        ))
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// Swaps `value` with the document object it targets; afterwards `value`
/// holds the previous state.
fn swap_in(doc: &mut Document, value: &mut Snapshot) -> Result<(), CommandError> {
    let id = value.target();
    let path = super::expect_kind(doc, id, value.target_kind())?;
    let missing = || CommandError::NotFound(id);
    match (path, value) {
        (ObjectPath::Table { table }, Snapshot::Table(v)) => {
            mem::swap(
                model_mut(doc)?.tables.get_mut(table).ok_or_else(missing)?,
                v,
            );
        }
        (ObjectPath::Column { table, column }, Snapshot::Column(v)) => {
            let t = model_mut(doc)?.tables.get_mut(table).ok_or_else(missing)?;
            mem::swap(t.columns.get_mut(column).ok_or_else(missing)?, v);
        }
        (ObjectPath::Measure { table, measure }, Snapshot::Measure(v)) => {
            let t = model_mut(doc)?.tables.get_mut(table).ok_or_else(missing)?;
            mem::swap(t.measures.get_mut(measure).ok_or_else(missing)?, v);
        }
        (ObjectPath::Relationship { relationship }, Snapshot::Relationship(v)) => {
            let m = model_mut(doc)?;
            mem::swap(
                m.relationships.get_mut(relationship).ok_or_else(missing)?,
                v,
            );
        }
        (ObjectPath::Expression { expression }, Snapshot::Expression(v)) => {
            let m = model_mut(doc)?;
            mem::swap(m.expressions.get_mut(expression).ok_or_else(missing)?, v);
        }
        (ObjectPath::Role { role }, Snapshot::Role(v)) => {
            mem::swap(model_mut(doc)?.roles.get_mut(role).ok_or_else(missing)?, v);
        }
        (
            ObjectPath::Visual {
                report,
                page,
                visual,
            },
            Snapshot::Visual(v),
        ) => {
            let p = doc
                .reports
                .get_mut(report)
                .and_then(|r| r.pages.get_mut(page))
                .ok_or_else(missing)?;
            mem::swap(p.visuals.get_mut(visual).ok_or_else(missing)?, v);
        }
        (ObjectPath::Report { report }, Snapshot::ReportFilters { filters, .. }) => {
            let r = doc.reports.get_mut(report).ok_or_else(missing)?;
            mem::swap(&mut r.filters, filters);
        }
        (ObjectPath::Page { report, page }, Snapshot::PageFilters { filters, .. }) => {
            let p = doc
                .reports
                .get_mut(report)
                .and_then(|r| r.pages.get_mut(page))
                .ok_or_else(missing)?;
            mem::swap(&mut p.filters, filters);
        }
        (path, value) => {
            return Err(CommandError::WrongKind {
                id,
                expected: value.target_kind(),
                actual: path.kind(),
            });
        }
    }
    Ok(())
}

fn named_event(id: ObjectId, old: &str, new: &str, differs: bool, events: &mut Vec<ChangeEvent>) {
    if old != new {
        events.push(ChangeEvent::ObjectRenamed {
            id,
            old: old.to_owned(),
            new: new.to_owned(),
        });
    } else if differs {
        events.push(ChangeEvent::ObjectChanged { id });
    }
}

/// Events for a child collection, matched by ID.
fn diff_children<T: PartialEq>(
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> (ObjectId, &str),
    events: &mut Vec<ChangeEvent>,
) {
    for n in new {
        let (id, name) = key(n);
        match old.iter().find(|o| key(o).0 == id) {
            Some(o) => named_event(id, key(o).1, name, o != n, events),
            None => events.push(ChangeEvent::ObjectAdded { id }),
        }
    }
    for o in old {
        let id = key(o).0;
        if !new.iter().any(|n| key(n).0 == id) {
            events.push(ChangeEvent::ObjectRemoved { id });
        }
    }
}

fn diff_events(old: &Snapshot, new: &Snapshot, events: &mut Vec<ChangeEvent>) {
    match (old, new) {
        (Snapshot::Table(a), Snapshot::Table(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
            diff_children(&a.columns, &b.columns, |c| (c.id, &c.name), events);
            diff_children(&a.measures, &b.measures, |m| (m.id, &m.name), events);
            diff_children(&a.partitions, &b.partitions, |p| (p.id, &p.name), events);
            diff_children(&a.hierarchies, &b.hierarchies, |h| (h.id, &h.name), events);
        }
        (Snapshot::Column(a), Snapshot::Column(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
        }
        (Snapshot::Measure(a), Snapshot::Measure(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
        }
        (Snapshot::Relationship(a), Snapshot::Relationship(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
        }
        (Snapshot::Expression(a), Snapshot::Expression(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
        }
        (Snapshot::Role(a), Snapshot::Role(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
        }
        (Snapshot::Visual(a), Snapshot::Visual(b)) => {
            named_event(a.id, &a.name, &b.name, a != b, events);
        }
        (
            Snapshot::ReportFilters {
                report: id,
                filters: a,
            },
            Snapshot::ReportFilters { filters: b, .. },
        )
        | (
            Snapshot::PageFilters {
                page: id,
                filters: a,
            },
            Snapshot::PageFilters { filters: b, .. },
        ) if a != b => {
            events.push(ChangeEvent::ObjectChanged { id: *id });
        }
        _ => {}
    }
}
