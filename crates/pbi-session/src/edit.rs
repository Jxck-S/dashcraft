//! DAX- and M-aware document edits.
//!
//! [`RenameObject`] renames a table, column, measure or shared expression
//! like the `pbi-model` rename commands (structured references in
//! relationships, roles, hierarchies, sort-by columns, report fields and
//! the query order follow) and also rewrites the DAX that refers to it:
//! measures, dynamic format strings, calculated columns, calculated tables,
//! calculation items and role filters. Renaming a query (a table with an M
//! partition, or a shared expression: query, parameter or function) also
//! rewrites the M that names it in partitions and shared expressions
//! ([`pbi_m::rename_query`]); a rename the M could not follow (the new name
//! is a step of a query that refers to the old one, or an M library name) is
//! refused. [`preview_rename`] reports what a rename would rewrite and what
//! it cannot (expressions that do not parse, ambiguous unqualified
//! references, refresh policies) without changing anything.
//!
//! [`ModelRenames`] finds the renames between two document states (by
//! [`ObjectId`]) so the session can rename the loaded data and its derived
//! columns instead of reloading.

use std::collections::HashMap;
use std::mem;

use pbi_dax::{NameChange, rename_references};
use pbi_engine::StorageRename;
use pbi_model::names::name_key;
use pbi_model::props::PropValue;
use pbi_model::{
    Applied, ColumnKind, Command, CommandError, CommandGroup, Document, ExpressionKind, Model,
    ObjectId, ObjectKind, ObjectPath, PartitionSource, RenameColumn, RenameExpression,
    RenameMeasure, RenameTable, ReplaceObjects, Snapshot, names_eq,
};
use serde::{Deserialize, Serialize};

use crate::derived::{DerivedColumn, DerivedEntry};
use crate::report::TableRefresh;

/// What a rename targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenameKind {
    Table,
    Column,
    Measure,
    /// A shared expression: M query, parameter or function.
    Expression,
}

/// One DAX expression a rename rewrites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaxRewrite {
    /// The object holding the expression, with the names after the rename
    /// (e.g. `measure 'Orders'[Total Sales]`).
    pub location: String,
    /// The property: `expression`, `formatStringDefinition`,
    /// `filterExpression`, …
    pub property: String,
    pub before: String,
    pub after: String,
}

/// One M expression a query rename rewrites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MRewrite {
    /// The object holding the expression, with the names after the rename:
    /// `table 'Orders'` (`table 'Orders' partition 'P'` when the table has
    /// several partitions) or `expression 'Clean Key'`.
    pub location: String,
    /// `source` (a partition) or `expression` (a shared expression).
    pub property: String,
    pub before: String,
    pub after: String,
}

/// What [`RenameObject`] does for one rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePreview {
    pub kind: RenameKind,
    /// The table: the renamed table's old name, or the table of the renamed
    /// column or measure (empty for an expression).
    pub table: String,
    pub old_name: String,
    pub new_name: String,
    /// DAX expressions that are rewritten.
    pub rewrites: Vec<DaxRewrite>,
    /// M expressions that are rewritten (query renames).
    #[serde(default)]
    pub m_rewrites: Vec<MRewrite>,
    /// References that are not rewritten and may need a manual fix.
    pub warnings: Vec<String>,
}

/// Renames a table, column, measure or shared expression and rewrites the
/// DAX and M that refer to it (see the [module docs](self)). One undo step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameObject {
    pub object: ObjectId,
    pub new_name: String,
}

impl Command for RenameObject {
    fn apply(&self, doc: &mut Document) -> Result<Applied, CommandError> {
        let plan = Plan::new(doc, self.object, &self.new_name)?;
        let renamed = plan.model_command().apply(doc)?;
        let rewritten = match plan.rewrite(doc) {
            Ok(rewritten) => rewritten,
            Err(err) => {
                // As below: reverting the rename just applied cannot fail.
                let _ = renamed.inverse.apply(doc);
                return Err(err);
            }
        };
        if rewritten.items.is_empty() {
            return Ok(renamed);
        }
        let replace = ReplaceObjects {
            label: self.label(),
            items: rewritten.items,
        };
        match replace.apply(doc) {
            Ok(applied) => {
                let mut events = renamed.events;
                events.extend(applied.events);
                Ok(Applied {
                    inverse: Box::new(CommandGroup {
                        label: self.label(),
                        commands: vec![applied.inverse, renamed.inverse],
                    }),
                    events,
                })
            }
            Err(err) => {
                // Reverting a rename just applied to this document cannot
                // fail; nothing better to do if it does.
                let _ = renamed.inverse.apply(doc);
                Err(err)
            }
        }
    }

    fn label(&self) -> String {
        format!("Rename to '{}'", self.new_name)
    }
}

/// Why DAX expression `text` does not parse (`line L, column C: …`), or
/// `None` when it does. Used to warn about an edit before it is evaluated.
pub fn dax_syntax_error(text: &str) -> Option<String> {
    pbi_dax::parse_expression(text)
        .err()
        .map(|err| err.render(text))
}

/// What [`RenameObject`] would rewrite in `doc`, and what it would leave.
///
/// # Errors
/// The rename is invalid (unknown or unsupported object, invalid or taken
/// name, no model) or refused (see [`RenameObject`]).
pub fn preview_rename(
    doc: &Document,
    object: ObjectId,
    new_name: &str,
) -> Result<RenamePreview, CommandError> {
    let plan = Plan::new(doc, object, new_name)?;
    let mut working = doc.clone();
    plan.model_command().apply(&mut working)?;
    let rewritten = plan.rewrite(&working)?;
    Ok(RenamePreview {
        kind: plan.kind,
        table: plan.table.clone(),
        old_name: plan.old.clone(),
        new_name: plan.new.clone(),
        rewrites: rewritten.rewrites,
        m_rewrites: rewritten.m_rewrites,
        warnings: rewritten.warnings.into_iter().map(|(_, w)| w).collect(),
    })
}

/// A rename and how unqualified `[name]` references are resolved.
struct Plan {
    kind: RenameKind,
    object: ObjectId,
    /// The renamed table, or the table of the renamed column or measure
    /// (`UNASSIGNED` and empty for an expression).
    table_id: ObjectId,
    table: String,
    old: String,
    new: String,
    /// The object is a query M can refer to: a table with an M partition
    /// or a shared M expression.
    query: bool,
    /// Column rename: a measure has the column's old name.
    measure_named_old: bool,
    /// Column rename: another table has a column with the old name.
    other_column_named_old: bool,
    /// Measure rename: some table has a column with the old name.
    column_named_old: bool,
    /// Finding references ([`dax_scan`]), not renaming: warnings say
    /// what may refer to the object instead of what is not rewritten.
    probe: bool,
}

impl Plan {
    fn new(doc: &Document, object: ObjectId, new_name: &str) -> Result<Plan, CommandError> {
        let model = doc.model.as_ref().ok_or(CommandError::NoModel)?;
        let path = doc.locate(object).ok_or(CommandError::NotFound(object))?;
        let missing = || CommandError::NotFound(object);
        let (kind, t, old, query) = match path {
            ObjectPath::Table { table } => {
                let t = model.tables.get(table).ok_or_else(missing)?;
                let query = t
                    .partitions
                    .iter()
                    .any(|p| matches!(p.source, PartitionSource::M { .. }));
                (RenameKind::Table, Some(t), t.name.clone(), query)
            }
            ObjectPath::Column { table, column } => {
                let t = model.tables.get(table).ok_or_else(missing)?;
                let c = t.columns.get(column).ok_or_else(missing)?;
                (RenameKind::Column, Some(t), c.name.clone(), false)
            }
            ObjectPath::Measure { table, measure } => {
                let t = model.tables.get(table).ok_or_else(missing)?;
                let m = t.measures.get(measure).ok_or_else(missing)?;
                (RenameKind::Measure, Some(t), m.name.clone(), false)
            }
            ObjectPath::Expression { expression } => {
                let e = model.expressions.get(expression).ok_or_else(missing)?;
                let query = e.kind == ExpressionKind::M;
                (RenameKind::Expression, None, e.name.clone(), query)
            }
            other => {
                return Err(CommandError::Unsupported {
                    kind: other.kind(),
                    operation: "rename",
                });
            }
        };
        let column_in = |table: &pbi_model::Table| table.column(&old).is_some();
        let table_id = t.map_or(ObjectId::UNASSIGNED, |t| t.id);
        Ok(Plan {
            kind,
            object,
            table_id,
            table: t.map(|t| t.name.clone()).unwrap_or_default(),
            query,
            measure_named_old: model
                .tables
                .iter()
                .any(|x| x.measures.iter().any(|m| names_eq(&m.name, &old))),
            other_column_named_old: model
                .tables
                .iter()
                .any(|x| x.id != table_id && column_in(x)),
            column_named_old: model.tables.iter().any(column_in),
            probe: false,
            old,
            new: new_name.to_owned(),
        })
    }

    fn model_command(&self) -> Box<dyn Command> {
        let new_name = self.new.clone();
        match self.kind {
            RenameKind::Table => Box::new(RenameTable {
                table: self.object,
                new_name,
            }),
            RenameKind::Column => Box::new(RenameColumn {
                column: self.object,
                new_name,
            }),
            RenameKind::Measure => Box::new(RenameMeasure {
                measure: self.object,
                new_name,
            }),
            RenameKind::Expression => Box::new(RenameExpression {
                expression: self.object,
                new_name,
            }),
        }
    }

    /// Whether an unqualified `[old]` in an expression evaluated on table
    /// `owner` (its home table or row context) can only mean the renamed
    /// column or measure.
    fn bare(&self, owner: ObjectId) -> bool {
        match self.kind {
            RenameKind::Table | RenameKind::Expression => false,
            RenameKind::Column => {
                !self.measure_named_old && (owner == self.table_id || !self.other_column_named_old)
            }
            RenameKind::Measure => !self.column_named_old,
        }
    }

    /// The DAX name change (`None` for an expression: DAX cannot name one).
    fn change(&self, bare: bool) -> Option<NameChange<'_>> {
        Some(match self.kind {
            RenameKind::Table => NameChange::Table {
                old: &self.old,
                new: &self.new,
            },
            RenameKind::Column => NameChange::Column {
                table: &self.table,
                old: &self.old,
                new: &self.new,
                bare,
            },
            RenameKind::Measure => NameChange::Measure {
                table: &self.table,
                old: &self.old,
                new: &self.new,
                bare,
            },
            RenameKind::Expression => return None,
        })
    }

    /// The DAX and M rewrites on `doc` (after the structured rename).
    ///
    /// # Errors
    /// [`CommandError::Rejected`] when the M cannot follow the rename.
    fn rewrite(&self, doc: &Document) -> Result<Rewritten, CommandError> {
        let mut out = Rewritten::default();
        let Some(model) = doc.model.as_ref() else {
            return Ok(out);
        };
        if self.query && !self.probe && pbi_m::library_names().contains(&self.new.as_str()) {
            return Err(CommandError::Rejected(format!(
                "cannot rename '{}' to '{}': M queries would call the library function '{}' instead",
                self.old, self.new, self.new
            )));
        }
        for table in &model.tables {
            let mut t = table.clone();
            let mut changed =
                self.kind != RenameKind::Expression && self.rewrite_table(&mut t, &mut out);
            if self.query {
                changed |= self.rewrite_m_table(&mut t, &mut out)?;
            }
            if changed {
                out.items.push(Snapshot::Table(t));
            }
        }
        if self.query {
            for e in model
                .expressions
                .iter()
                .filter(|e| e.kind == ExpressionKind::M)
            {
                let mut x = e.clone();
                let location = format!("expression '{}'", e.name);
                let owner = ObjectId::UNASSIGNED;
                if self.m_text(&mut x.expression, owner, &location, "expression", &mut out)? {
                    out.items.push(Snapshot::Expression(x));
                }
            }
        }
        for role in &model.roles {
            let mut r = role.clone();
            let mut changed = false;
            for p in &mut r.table_permissions {
                let owner = model.table(&p.table).map_or(ObjectId::UNASSIGNED, |t| t.id);
                let location = format!("role '{}' on '{}'", role.name, p.table);
                if let Some(filter) = &mut p.filter_expression {
                    changed |= self.text(filter, owner, &location, "filterExpression", &mut out);
                }
            }
            if changed {
                out.items.push(Snapshot::Role(r));
            }
        }
        Ok(out)
    }

    /// Rewrites the M partitions of `t` naming the renamed query; `true` if
    /// one changed.
    fn rewrite_m_table(
        &self,
        t: &mut pbi_model::Table,
        out: &mut Rewritten,
    ) -> Result<bool, CommandError> {
        let (owner, name) = (t.id, t.name.clone());
        let several = t.partitions.len() > 1;
        let mut changed = false;
        for p in &mut t.partitions {
            if let PartitionSource::M { expression } = &mut p.source {
                let location = if several {
                    format!("table '{name}' partition '{}'", p.name)
                } else {
                    format!("table '{name}'")
                };
                changed |= self.m_text(expression, owner, &location, "source", out)?;
            }
        }
        if let Some(policy) = t.extra.get("refreshPolicy")
            && serde_json::to_string(policy).is_ok_and(|text| m_mentions(&text, &self.old))
        {
            let warning = if self.probe {
                format!(
                    "the refresh policy of table '{name}' may refer to '{}'",
                    self.old
                )
            } else {
                format!(
                    "the refresh policy of table '{name}' mentions '{}'; it is not rewritten",
                    self.old
                )
            };
            out.warnings.push((owner, warning));
        }
        Ok(changed)
    }

    /// Rewrites one M expression in place; `true` if it changed. Probing,
    /// only warns about a mention.
    fn m_text(
        &self,
        text: &mut String,
        owner: ObjectId,
        location: &str,
        property: &str,
        out: &mut Rewritten,
    ) -> Result<bool, CommandError> {
        if self.probe {
            if m_mentions(text, &self.old) {
                let warning = format!("the M query of {location} refers to '{}'", self.old);
                out.warnings.push((owner, warning));
            }
            return Ok(false);
        }
        match pbi_m::rename_query(text, &self.old, &self.new) {
            Ok(None) => Ok(false),
            Ok(Some(after)) => {
                let before = mem::replace(text, after.clone());
                out.m_rewrites.push(MRewrite {
                    location: location.to_owned(),
                    property: property.to_owned(),
                    before,
                    after,
                });
                Ok(true)
            }
            Err(pbi_m::RenameError::Parse(_)) => {
                if m_mentions(text, &self.old) {
                    out.warnings.push((
                        owner,
                        format!(
                            "the M query of {location} does not parse; references to '{}' in it were not rewritten",
                            self.old
                        ),
                    ));
                }
                Ok(false)
            }
            Err(pbi_m::RenameError::Captured { .. }) => Err(CommandError::Rejected(format!(
                "cannot rename '{}' to '{}': the M query of {location} refers to '{}' where '{}' is one of its own steps, parameters or fields",
                self.old, self.new, self.old, self.new
            ))),
        }
    }

    fn rewrite_table(&self, t: &mut pbi_model::Table, out: &mut Rewritten) -> bool {
        let owner = t.id;
        let name = t.name.clone();
        let mut changed = false;
        for m in &mut t.measures {
            let location = format!("measure '{name}'[{}]", m.name);
            changed |= self.text(&mut m.expression, owner, &location, "expression", out);
            if let Some(text) = m
                .extra
                .get_mut("formatStringDefinition")
                .and_then(prop_text_mut)
            {
                changed |= self.text(text, owner, &location, "formatStringDefinition", out);
            }
        }
        for c in &mut t.columns {
            if let ColumnKind::Calculated { expression } = &mut c.kind {
                let location = format!("column '{name}'[{}]", c.name);
                changed |= self.text(expression, owner, &location, "expression", out);
            }
        }
        for p in &mut t.partitions {
            if let PartitionSource::Calculated { expression } = &mut p.source {
                let location = format!("calculated table '{name}'");
                changed |= self.text(expression, owner, &location, "expression", out);
            }
        }
        if let Some(group) = &mut t.calculation_group {
            for item in &mut group.calculation_items {
                let location = format!("calculation item '{name}'[{}]", item.name);
                changed |= self.text(&mut item.expression, owner, &location, "expression", out);
                if let Some(text) = &mut item.format_string_definition {
                    changed |= self.text(text, owner, &location, "formatStringDefinition", out);
                }
            }
            let location = format!("calculation group '{name}'");
            for (property, selection) in [
                (
                    "multipleOrEmptySelectionExpression",
                    &mut group.multiple_or_empty_selection_expression,
                ),
                ("noSelectionExpression", &mut group.no_selection_expression),
            ] {
                let Some(selection) = selection else {
                    continue;
                };
                changed |= self.text(&mut selection.expression, owner, &location, property, out);
                if let Some(text) = &mut selection.format_string_definition {
                    let property = format!("{property}.formatStringDefinition");
                    changed |= self.text(text, owner, &location, &property, out);
                }
            }
        }
        changed
    }

    /// Rewrites one expression in place; `true` if it changed.
    fn text(
        &self,
        text: &mut String,
        owner: ObjectId,
        location: &str,
        property: &str,
        out: &mut Rewritten,
    ) -> bool {
        let bare = self.bare(owner);
        let Some(change) = self.change(bare) else {
            return false;
        };
        let rewritten = match rename_references(text, &change) {
            Ok(rewritten) => rewritten,
            Err(_) => {
                if self.probe {
                    out.unparsed.push(DaxText {
                        owner,
                        location: location.to_owned(),
                        property: property.to_owned(),
                        text: text.clone(),
                    });
                }
                if text.to_lowercase().contains(&self.old.to_lowercase()) {
                    let warning = if self.probe {
                        format!(
                            "{location} {property} does not parse and mentions '{}'; it may refer to it",
                            self.old
                        )
                    } else {
                        format!(
                            "{location} {property} does not parse; references to '{}' in it were not rewritten",
                            self.old
                        )
                    };
                    out.warnings.push((owner, warning));
                }
                return false;
            }
        };
        if !bare
            && self.kind != RenameKind::Table
            && let Some(change) = self.change(true)
            && let Ok(Some(all)) = rename_references(text, &change)
            && Some(&all) != rewritten.as_ref()
        {
            let warning = if self.probe {
                format!(
                    "{location} {property}: unqualified [{}] references may refer to it or to another object",
                    self.old
                )
            } else {
                format!(
                    "{location} {property}: unqualified [{}] references were not rewritten because they may refer to another object",
                    self.old
                )
            };
            out.warnings.push((owner, warning));
        }
        let Some(after) = rewritten else {
            return false;
        };
        let before = mem::replace(text, after.clone());
        out.owners.push(owner);
        out.rewrites.push(DaxRewrite {
            location: location.to_owned(),
            property: property.to_owned(),
            before,
            after,
        });
        true
    }
}

/// A DAX expression kept outside the document (a session derived column)
/// that [`dax_scan`] searches too.
pub(crate) struct ExtraDax<'a> {
    /// The expression's home table.
    pub table: &'a str,
    pub location: String,
    pub property: &'static str,
    pub text: &'a str,
}

/// One DAX expression of the document, where it lives and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaxText {
    /// The expression's home table (`UNASSIGNED` when unknown).
    pub owner: ObjectId,
    pub location: String,
    pub property: String,
    pub text: String,
}

/// What [`dax_scan`] finds.
pub(crate) struct DaxScan {
    /// The expressions that refer to the object, with their text.
    pub references: Vec<DaxText>,
    /// With the home table of the expression they are about (`UNASSIGNED`
    /// for M expressions).
    pub warnings: Vec<(ObjectId, String)>,
    /// Every expression that does not parse.
    pub unparsed: Vec<DaxText>,
}

/// The DAX expressions of `doc` (and `extra`) that refer to table, column or
/// measure `object`, with their texts and home tables; warnings about
/// references that cannot be confirmed (expressions that do not parse,
/// ambiguous unqualified references, M queries naming a table); and every
/// expression that does not parse.
pub(crate) fn dax_scan(
    doc: &Document,
    object: ObjectId,
    extra: &[ExtraDax<'_>],
) -> Result<DaxScan, CommandError> {
    // Any name other than the old one: a reference shows up as a rewrite.
    const PROBE: &str = "\u{e000}";
    let unsupported = |kind| CommandError::Unsupported {
        kind,
        operation: "dependents",
    };
    let mut plan = Plan::new(doc, object, PROBE).map_err(|err| match err {
        CommandError::Unsupported { kind, .. } => unsupported(kind),
        other => other,
    })?;
    if plan.kind == RenameKind::Expression {
        return Err(unsupported(ObjectKind::Expression));
    }
    plan.probe = true;
    let mut out = plan.rewrite(doc)?;
    if let Some(model) = doc.model.as_ref() {
        for e in extra {
            let owner = model.table(e.table).map_or(ObjectId::UNASSIGNED, |t| t.id);
            let mut text = e.text.to_owned();
            plan.text(&mut text, owner, &e.location, e.property, &mut out);
        }
    }
    let references = out
        .rewrites
        .into_iter()
        .zip(out.owners)
        .map(|(r, owner)| DaxText {
            owner,
            location: r.location,
            property: r.property,
            text: r.before,
        })
        .collect();
    Ok(DaxScan {
        references,
        warnings: out.warnings,
        unparsed: out.unparsed,
    })
}

#[derive(Default)]
struct Rewritten {
    items: Vec<Snapshot>,
    rewrites: Vec<DaxRewrite>,
    m_rewrites: Vec<MRewrite>,
    /// The home table of each rewrite.
    owners: Vec<ObjectId>,
    /// With the home table of the expression they are about.
    warnings: Vec<(ObjectId, String)>,
    /// Probe only: the expressions that do not parse.
    unparsed: Vec<DaxText>,
}

/// The text of a DAX expression kept in an untyped property.
fn prop_text_mut(value: &mut PropValue) -> Option<&mut String> {
    match value {
        PropValue::Expression(s) | PropValue::Text(s) => Some(s),
        PropValue::Object(o) => o.expression.as_mut(),
        PropValue::Json(serde_json::Value::String(s)) => Some(s),
        PropValue::Json(serde_json::Value::Object(map)) => match map.get_mut("expression") {
            Some(serde_json::Value::String(s)) => Some(s),
            _ => None,
        },
        PropValue::List(items) => items.first_mut().and_then(prop_text_mut),
        PropValue::Json(_) | PropValue::Bool(_) => None,
    }
}

/// Whether M `src` has an identifier `name` (`name` or `#"name"`; M names
/// are case-sensitive). Text that does not lex is searched as plain text.
fn m_mentions(src: &str, name: &str) -> bool {
    match pbi_m::tokenize(src) {
        Ok(lexed) => lexed.tokens.iter().any(|t| {
            matches!(&t.kind, pbi_m::TokenKind::Identifier(s) | pbi_m::TokenKind::QuotedIdentifier(s) if s == name)
        }),
        Err(_) => src.contains(name),
    }
}

/// Table, column and measure renames between two states of a model,
/// matched by [`ObjectId`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelRenames {
    /// `(old, new)` table names.
    pub tables: Vec<(String, String)>,
    /// `(table, old, new)` with the table's new name.
    pub columns: Vec<(String, String, String)>,
    /// `(table, old, new)` with the table's new name.
    pub measures: Vec<(String, String, String)>,
}

impl ModelRenames {
    pub fn between(old: &Model, new: &Model) -> ModelRenames {
        let mut out = ModelRenames::default();
        let old_tables: HashMap<ObjectId, &pbi_model::Table> = old
            .tables
            .iter()
            .filter(|t| t.id.is_assigned())
            .map(|t| (t.id, t))
            .collect();
        for table in &new.tables {
            let Some(before) = old_tables.get(&table.id).filter(|_| table.id.is_assigned()) else {
                continue;
            };
            if before.name != table.name {
                out.tables.push((before.name.clone(), table.name.clone()));
            }
            for c in &table.columns {
                if let Some(o) = before
                    .columns
                    .iter()
                    .find(|o| o.id == c.id && c.id.is_assigned())
                    && o.name != c.name
                {
                    out.columns
                        .push((table.name.clone(), o.name.clone(), c.name.clone()));
                }
            }
            for m in &table.measures {
                if let Some(o) = before
                    .measures
                    .iter()
                    .find(|o| o.id == m.id && m.id.is_assigned())
                    && o.name != m.name
                {
                    out.measures
                        .push((table.name.clone(), o.name.clone(), m.name.clone()));
                }
            }
        }
        out
    }

    /// `true` when nothing was renamed.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty() && self.columns.is_empty() && self.measures.is_empty()
    }

    /// The renames of stored tables and columns.
    pub fn storage(&self) -> Vec<StorageRename> {
        let tables = self.tables.iter().map(|(old, new)| StorageRename::Table {
            old: old.clone(),
            new: new.clone(),
        });
        let columns = self
            .columns
            .iter()
            .map(|(table, old, new)| StorageRename::Column {
                table: table.clone(),
                old: old.clone(),
                new: new.clone(),
            });
        tables.chain(columns).collect()
    }

    /// Refuses renames onto the name of a derived column: a column of the
    /// same table, or a measure (DAX could not tell them apart).
    pub fn check_derived(&self, entries: &[DerivedEntry]) -> Result<(), CommandError> {
        let table_of = |e: &DerivedEntry| {
            self.tables
                .iter()
                .find(|(old, _)| names_eq(old, &e.info.table))
                .map_or(e.info.table.clone(), |(_, new)| new.clone())
        };
        let taken = |table: Option<&str>, name: &str| {
            entries.iter().find(|e| {
                names_eq(&e.info.name, name) && table.is_none_or(|t| names_eq(&table_of(e), t))
            })
        };
        let clash = self
            .columns
            .iter()
            .find_map(|(table, _, new)| taken(Some(table), new))
            .or_else(|| {
                self.measures
                    .iter()
                    .find_map(|(_, _, new)| taken(None, new))
            });
        match clash {
            Some(e) => Err(CommandError::Rejected(format!(
                "the name '{}' is already used by the derived column '{}'[{}]",
                e.info.name, e.info.table, e.info.name
            ))),
            None => Ok(()),
        }
    }

    /// Moves derived columns to renamed tables and rewrites their source
    /// columns and DAX. `model` is the new model.
    pub fn apply_to_derived(&self, entries: &mut [DerivedEntry], model: &Model) {
        for entry in entries.iter_mut() {
            if let Some((_, new)) = self
                .tables
                .iter()
                .find(|(old, _)| names_eq(old, &entry.info.table))
            {
                entry.info.table.clone_from(new);
            }
            let table = entry.info.table.clone();
            match &mut entry.column {
                DerivedColumn::People { source_column } => {
                    if let Some((_, _, new)) = self
                        .columns
                        .iter()
                        .find(|(t, old, _)| names_eq(t, &table) && names_eq(old, source_column))
                    {
                        source_column.clone_from(new);
                        entry.info.source_column = Some(new.clone());
                    }
                }
                DerivedColumn::Expression { dax } => {
                    if self.rewrite_dax(dax, &table, model) {
                        entry.info.dax = Some(dax.clone());
                    }
                }
                DerivedColumn::Array(_) => {}
            }
        }
    }

    /// Rewrites the references in `dax`, evaluated on `table`, to every
    /// rename; `true` if it changed.
    pub fn rewrite_dax(&self, dax: &mut String, table: &str, model: &Model) -> bool {
        let mut changed = false;
        let mut apply = |change: NameChange<'_>| {
            if let Ok(Some(text)) = rename_references(dax, &change) {
                *dax = text;
                changed = true;
            }
        };
        for (old, new) in &self.tables {
            apply(NameChange::Table { old, new });
        }
        for (owner, old, new) in &self.columns {
            let ambiguous = model
                .tables
                .iter()
                .any(|t| t.measures.iter().any(|m| names_eq(&m.name, old)))
                || (!names_eq(owner, table)
                    && model
                        .tables
                        .iter()
                        .any(|t| !names_eq(&t.name, owner) && t.column(old).is_some()));
            apply(NameChange::Column {
                table: owner,
                old,
                new,
                bare: !ambiguous,
            });
        }
        for (owner, old, new) in &self.measures {
            let ambiguous = model.tables.iter().any(|t| t.column(old).is_some());
            apply(NameChange::Measure {
                table: owner,
                old,
                new,
                bare: !ambiguous,
            });
        }
        changed
    }
}

/// Whether the engine must re-sync with `new` (tables, columns,
/// relationships or roles changed; renames are handled separately).
pub(crate) fn structure_changed(old: &Model, new: &Model) -> bool {
    old.relationships != new.relationships
        || old.roles != new.roles
        || old.tables.len() != new.tables.len()
        || old.tables.iter().zip(&new.tables).any(|(a, b)| {
            a.name != b.name
                || a.columns.len() != b.columns.len()
                || a.columns
                    .iter()
                    .zip(&b.columns)
                    .any(|(x, y)| x.name != y.name)
        })
}

/// Re-keys the per-table refresh results after table renames
/// (simultaneously, so a swap works) and renames their column results.
pub(crate) fn rename_results(map: &mut HashMap<String, TableRefresh>, renames: &ModelRenames) {
    let moved: Vec<(String, TableRefresh)> = renames
        .tables
        .iter()
        .filter_map(|(old, new)| map.remove(&name_key(old)).map(|v| (new.clone(), v)))
        .collect();
    for (new, mut result) in moved {
        result.name.clone_from(&new);
        map.insert(name_key(&new), result);
    }
    for (table, old, new) in &renames.columns {
        if let Some(result) = map.get_mut(&name_key(table)) {
            for c in &mut result.columns {
                if names_eq(&c.name, old) {
                    c.name.clone_from(new);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::{Column, DataType, Measure, Table};

    fn doc() -> Document {
        let mut orders = Table::new("Orders");
        orders
            .columns
            .push(Column::data("Line Amount", DataType::Double));
        orders.columns.push(Column::calculated(
            "Big",
            DataType::Boolean,
            "[Line Amount] > 100",
        ));
        orders
            .measures
            .push(Measure::new("Total Sales", "SUM ( Orders[Line Amount] )"));
        orders
            .measures
            .push(Measure::new("Twice", "[Total Sales] * 2"));
        let mut returns = Table::new("Returns");
        returns
            .columns
            .push(Column::data("Line Amount", DataType::Double));
        returns.columns.push(Column::calculated(
            "Ratio",
            DataType::Double,
            "[Line Amount] / [Total Sales]",
        ));
        returns
            .measures
            .push(Measure::new("Broken", "SUM(Orders[Line Amount]"));
        let mut doc = Document {
            model: Some(Model {
                tables: vec![orders, returns],
                ..Model::default()
            }),
            ..Document::default()
        };
        doc.assign_ids();
        doc
    }

    fn id(doc: &Document, f: impl Fn(&Model) -> Option<ObjectId>) -> ObjectId {
        f(doc.model.as_ref().unwrap()).unwrap()
    }

    #[test]
    fn column_rename_rewrites_qualified_and_unambiguous_references() {
        let mut d = doc();
        let column = id(&d, |m| m.column("Orders", "Line Amount").map(|c| c.id));
        let preview = preview_rename(&d, column, "Net Amount").unwrap();
        assert_eq!(preview.kind, RenameKind::Column);
        assert_eq!(preview.rewrites.len(), 2, "{preview:#?}");
        // The other table also has [Line Amount]: its bare reference stays.
        assert!(
            preview
                .warnings
                .iter()
                .any(|w| w.contains("'Returns'[Broken]"))
        );
        let original = d.clone();
        let applied = RenameObject {
            object: column,
            new_name: "Net Amount".into(),
        }
        .apply(&mut d)
        .unwrap();
        let model = d.model.as_ref().unwrap();
        assert_eq!(
            model.measure("Orders", "Total Sales").unwrap().expression,
            "SUM ( Orders[Net Amount] )"
        );
        let calc = |t: &str, c: &str| match &model.column(t, c).unwrap().kind {
            ColumnKind::Calculated { expression } => expression.clone(),
            _ => String::new(),
        };
        assert_eq!(calc("Orders", "Big"), "[Net Amount] > 100");
        assert_eq!(calc("Returns", "Ratio"), "[Line Amount] / [Total Sales]");
        applied.inverse.apply(&mut d).unwrap();
        assert_eq!(d, original);
    }

    #[test]
    fn table_and_measure_renames_round_trip() {
        let mut d = doc();
        let original = d.clone();
        let table = id(&d, |m| m.table("Orders").map(|t| t.id));
        let applied = RenameObject {
            object: table,
            new_name: "Sales".into(),
        }
        .apply(&mut d)
        .unwrap();
        let model = d.model.as_ref().unwrap();
        assert_eq!(
            model.measure("Sales", "Total Sales").unwrap().expression,
            "SUM ( Sales[Line Amount] )"
        );
        let redo = applied.inverse.apply(&mut d).unwrap();
        assert_eq!(d, original);
        redo.inverse.apply(&mut d).unwrap();
        assert!(d.model.as_ref().unwrap().table("Sales").is_some());

        let mut d = doc();
        let measure = id(&d, |m| m.measure("Orders", "Total Sales").map(|x| x.id));
        RenameObject {
            object: measure,
            new_name: "Revenue".into(),
        }
        .apply(&mut d)
        .unwrap();
        let model = d.model.as_ref().unwrap();
        assert_eq!(
            model.measure("Orders", "Twice").unwrap().expression,
            "[Revenue] * 2"
        );
        let ratio = &model.column("Returns", "Ratio").unwrap().kind;
        assert!(
            matches!(ratio, ColumnKind::Calculated { expression } if expression == "[Line Amount] / [Revenue]")
        );
    }

    #[test]
    fn invalid_renames_leave_the_document_unchanged() {
        let mut d = doc();
        let original = d.clone();
        let column = id(&d, |m| m.column("Orders", "Line Amount").map(|c| c.id));
        let err = RenameObject {
            object: column,
            new_name: "Big".into(),
        }
        .apply(&mut d)
        .unwrap_err();
        assert!(matches!(err, CommandError::NameConflict { .. }));
        assert_eq!(d, original);
        assert!(matches!(
            preview_rename(&d, ObjectId(999_999), "X"),
            Err(CommandError::NotFound(_))
        ));
    }

    fn m_doc() -> Document {
        let m_table = |name: &str, source: &str| {
            let mut t = Table::new(name);
            t.columns.push(Column::data("Key", DataType::String));
            t.partitions.push(pbi_model::Partition {
                name: name.to_owned(),
                source: PartitionSource::M {
                    expression: source.to_owned(),
                },
                ..pbi_model::Partition::default()
            });
            t
        };
        let orders = m_table(
            "Orders",
            "let\n    Source = Csv.Document(Files),\n    Clean = fnClean(Source)\nin\n    Clean",
        );
        let summary = m_table(
            "Summary",
            "let\n    Rows = Orders,\n    Count = Table.RowCount(Rows)\nin\n    #table({\"N\"}, {{Count}})",
        );
        let broken = m_table("Broken", "let x = Orders in");
        let mut calc = Table::new("Orders Calc");
        calc.partitions.push(pbi_model::Partition {
            name: "Orders Calc".into(),
            source: PartitionSource::Calculated {
                expression: "Orders".into(),
            },
            ..pbi_model::Partition::default()
        });
        let mut doc = Document {
            model: Some(Model {
                tables: vec![orders, summary, broken, calc],
                expressions: vec![
                    pbi_model::NamedExpression::m(
                        "Files",
                        r#""C:\data" meta [IsParameterQuery=true, Type="Text"]"#,
                    ),
                    pbi_model::NamedExpression::m(
                        "fnClean",
                        "(t as table) as table => Table.Distinct(t)",
                    ),
                    pbi_model::NamedExpression::m("Everything", "fnClean(Orders)"),
                ],
                ..Model::default()
            }),
            ..Document::default()
        };
        doc.assign_ids();
        doc
    }

    fn partition(doc: &Document, table: &str) -> String {
        match &doc.model.as_ref().unwrap().table(table).unwrap().partitions[0].source {
            PartitionSource::M { expression } | PartitionSource::Calculated { expression } => {
                expression.clone()
            }
            _ => String::new(),
        }
    }

    #[test]
    fn query_renames_rewrite_m_references() {
        let mut d = m_doc();
        let original = d.clone();
        let table = id(&d, |m| m.table("Orders").map(|t| t.id));
        let preview = preview_rename(&d, table, "Sales Orders").unwrap();
        assert_eq!(preview.kind, RenameKind::Table);
        let rewritten: Vec<_> = preview
            .m_rewrites
            .iter()
            .map(|r| (r.location.as_str(), r.property.as_str()))
            .collect();
        assert_eq!(
            rewritten,
            [
                ("table 'Summary'", "source"),
                ("expression 'Everything'", "expression")
            ]
        );
        assert_eq!(
            preview.warnings,
            [
                "the M query of table 'Broken' does not parse; references to 'Orders' in it were not rewritten"
            ]
        );
        let applied = RenameObject {
            object: table,
            new_name: "Sales Orders".into(),
        }
        .apply(&mut d)
        .unwrap();
        assert!(partition(&d, "Summary").starts_with("let\n    Rows = #\"Sales Orders\","));
        // DAX in a calculated table follows; the M query of the renamed table
        // and the unparsable one stay as they were.
        assert_eq!(partition(&d, "Orders Calc"), "'Sales Orders'");
        assert_eq!(
            partition(&d, "Sales Orders"),
            partition(&original, "Orders")
        );
        assert_eq!(partition(&d, "Broken"), "let x = Orders in");
        let model = d.model.as_ref().unwrap();
        assert_eq!(
            model.expressions[2].expression,
            "fnClean(#\"Sales Orders\")"
        );
        applied.inverse.apply(&mut d).unwrap();
        assert_eq!(d, original);

        // A shared function: its callers follow, its own text does not
        // change; a case-only rename rewrites too (M is case-sensitive).
        let function = d.model.as_ref().unwrap().expressions[1].id;
        for new_name in ["Clean Key", "FnClean"] {
            let mut d = original.clone();
            RenameObject {
                object: function,
                new_name: new_name.into(),
            }
            .apply(&mut d)
            .unwrap();
            let call = pbi_m::refs::identifier(new_name);
            assert!(partition(&d, "Orders").contains(&format!("Clean = {call}(Source)")));
            let model = d.model.as_ref().unwrap();
            assert_eq!(model.expressions[1].name, new_name);
            assert_eq!(
                model.expressions[1].expression,
                original.model.as_ref().unwrap().expressions[1].expression
            );
            assert_eq!(model.expressions[2].expression, format!("{call}(Orders)"));
        }

        // A calculated table is not a query: M is left alone.
        let mut d = original.clone();
        let calc = id(&d, |m| m.table("Orders Calc").map(|t| t.id));
        let preview = preview_rename(&d, calc, "Orders").unwrap_err();
        assert!(matches!(preview, CommandError::NameConflict { .. }));
        let preview = preview_rename(&d, calc, "Copy").unwrap();
        assert!(preview.m_rewrites.is_empty() && preview.warnings.is_empty());
        RenameObject {
            object: calc,
            new_name: "Copy".into(),
        }
        .apply(&mut d)
        .unwrap();
    }

    #[test]
    fn query_renames_the_m_cannot_follow_are_refused() {
        let mut d = m_doc();
        let original = d.clone();
        let files = d.model.as_ref().unwrap().expressions[0].id;
        let orders = id(&d, |m| m.table("Orders").map(|t| t.id));
        for (object, new_name, message) in [
            (
                files,
                "Source",
                "cannot rename 'Files' to 'Source': the M query of table 'Orders' refers to 'Files' where 'Source' is one of its own steps, parameters or fields",
            ),
            (
                orders,
                "Rows",
                "cannot rename 'Orders' to 'Rows': the M query of table 'Summary' refers to 'Orders' where 'Rows' is one of its own steps, parameters or fields",
            ),
            (
                files,
                "Table.RowCount",
                "cannot rename 'Files' to 'Table.RowCount': M queries would call the library function 'Table.RowCount' instead",
            ),
        ] {
            let err = preview_rename(&d, object, new_name).unwrap_err();
            assert_eq!(
                err.to_string(),
                CommandError::Rejected(message.into()).to_string()
            );
            let err = RenameObject {
                object,
                new_name: new_name.into(),
            }
            .apply(&mut d)
            .unwrap_err();
            assert!(matches!(err, CommandError::Rejected(_)), "{err}");
            assert_eq!(d, original);
        }
        // A let step is in scope in every step of its let…
        RenameObject {
            object: files,
            new_name: "Clean".into(),
        }
        .apply(&mut d)
        .unwrap_err();
        // …but a step of a query that does not refer to the old name is no
        // obstacle.
        let mut d = original.clone();
        RenameObject {
            object: files,
            new_name: "Count".into(),
        }
        .apply(&mut d)
        .unwrap();
        assert!(partition(&d, "Orders").contains("Csv.Document(Count)"));
        // Finding references is DAX only; a query rename's M warnings are
        // still reported when probing a table.
        assert!(matches!(
            dax_scan(&original, files, &[]),
            Err(CommandError::Unsupported { .. })
        ));
        let scan = dax_scan(&original, orders, &[]).ok().unwrap();
        assert!(
            scan.warnings
                .iter()
                .any(|(_, w)| w == "the M query of table 'Summary' refers to 'Orders'")
        );
    }

    #[test]
    fn model_renames_are_matched_by_id() {
        let before = doc();
        let mut after = before.clone();
        let table = id(&after, |m| m.table("Orders").map(|t| t.id));
        RenameTable {
            table,
            new_name: "Sales".into(),
        }
        .apply(&mut after)
        .unwrap();
        let renames = ModelRenames::between(
            before.model.as_ref().unwrap(),
            after.model.as_ref().unwrap(),
        );
        assert_eq!(renames.tables, [("Orders".to_owned(), "Sales".to_owned())]);
        assert!(renames.columns.is_empty() && renames.measures.is_empty());

        let result = |name: &str| TableRefresh {
            name: name.to_owned(),
            kind: crate::report::RefreshKind::Manual,
            rows: 0,
            duration_ms: 0,
            status: crate::report::TableStatus::Ok,
            columns: Vec::new(),
        };
        let mut map = HashMap::from([
            ("orders".to_owned(), result("Orders")),
            ("sales".to_owned(), result("Sales")),
        ]);
        let swap = ModelRenames {
            tables: vec![
                ("Orders".into(), "Sales".into()),
                ("Sales".into(), "Orders".into()),
            ],
            ..ModelRenames::default()
        };
        rename_results(&mut map, &swap);
        assert_eq!(map["sales"].name, "Sales");
        assert_eq!(map["orders"].name, "Orders");
        assert_eq!(map.len(), 2);
    }
}
