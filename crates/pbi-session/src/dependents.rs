//! What refers to a table, column, measure, hierarchy or relationship.
//!
//! [`object_dependents`] lists the places that would break if the object
//! were deleted: DAX expressions (measures, dynamic format strings,
//! calculated columns and tables, calculation items, role filters), model
//! structure (relationship ends, hierarchy levels, sort-by columns, role
//! table permissions) and report fields (visual projections, sorts, filters
//! and formatting, page and report filters) and report-level measures
//! (their DAX, and the table they are shown in). For a relationship, the
//! dependents are the DAX expressions whose `USERELATIONSHIP` or
//! `CROSSFILTER` calls name both of its columns; for a user hierarchy, the
//! report fields that use one of its levels (DAX cannot name a hierarchy).
//! [`Session::object_dependents`] adds the session's derived DAX columns.
//! References that cannot be confirmed are reported as warnings.

use pbi_model::{
    ColumnKind, ColumnRef, CommandError, Document, FieldExpr, Model, ObjectId, ObjectPath,
    Relationship, Rename, Visual, names_eq,
};
use serde::{Deserialize, Serialize};

use crate::DerivedKind;
use crate::edit::{ExtraDax, dax_scan, report_measure_location};
use crate::session::Session;

/// One object property that refers to the inspected object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependent {
    /// E.g. `measure 'Orders'[Total Sales]`, `visual 'card1' on page
    /// 'Overview' of report 'Sales'`.
    pub location: String,
    /// E.g. `expression`, `filterExpression`, `fields`, `filters`,
    /// `fromColumn`, `sortByColumn`.
    pub property: String,
}

/// The result of [`object_dependents`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependents {
    pub dependents: Vec<Dependent>,
    /// References that may exist but cannot be confirmed.
    pub warnings: Vec<String>,
}

/// What in `doc` refers to table, column, measure, hierarchy or relationship
/// `object`
/// (see the [module docs](self)).
///
/// # Errors
/// `object` is unknown or not a table, column, measure, hierarchy or
/// relationship, or
/// there is no model.
pub fn object_dependents(doc: &Document, object: ObjectId) -> Result<Dependents, CommandError> {
    find(doc, object, &[], None)
}

/// What in `doc` breaks when table, column or measure `object` is deleted:
/// [`object_dependents`] without what goes with it. A table takes its own
/// DAX (measures, calculated columns, its expression, calculation items,
/// role filters on it), its relationships and its role table permissions
/// along; references to its measures elsewhere break too.
///
/// # Errors
/// See [`object_dependents`].
pub fn delete_dependents(doc: &Document, object: ObjectId) -> Result<Dependents, CommandError> {
    deleted(doc, object, &[])
}

impl Session {
    /// [`object_dependents`] on the current document, including the DAX of
    /// the session's derived columns.
    ///
    /// # Errors
    /// See [`object_dependents`].
    pub fn object_dependents(&self, object: ObjectId) -> Result<Dependents, CommandError> {
        let derived = self.derived_columns();
        find(&self.document(), object, &derived_dax(&derived), None)
    }

    /// [`delete_dependents`] on the current document, including the DAX of
    /// the session's derived columns.
    ///
    /// # Errors
    /// See [`object_dependents`].
    pub fn delete_dependents(&self, object: ObjectId) -> Result<Dependents, CommandError> {
        let derived = self.derived_columns();
        deleted(&self.document(), object, &derived_dax(&derived))
    }
}

fn derived_dax(derived: &[crate::DerivedColumnInfo]) -> Vec<ExtraDax<'_>> {
    derived
        .iter()
        .filter(|d| d.kind == DerivedKind::Expression)
        .filter_map(|d| {
            Some(ExtraDax {
                table: &d.table,
                location: format!("derived column '{}'[{}]", d.table, d.name),
                property: "expression",
                text: d.dax.as_deref()?,
            })
        })
        .collect()
}

fn deleted(
    doc: &Document,
    object: ObjectId,
    extra: &[ExtraDax<'_>],
) -> Result<Dependents, CommandError> {
    let Some(ObjectPath::Table { table }) = doc.locate(object) else {
        return find(doc, object, extra, None);
    };
    let model = doc.model.as_ref().ok_or(CommandError::NoModel)?;
    let t = model
        .tables
        .get(table)
        .ok_or(CommandError::NotFound(object))?;
    let mut out = Dependents::default();
    for id in std::iter::once(object).chain(t.measures.iter().map(|m| m.id)) {
        let found = find(doc, id, extra, Some(object))?;
        for d in found.dependents {
            if !out.dependents.contains(&d) {
                out.dependents.push(d);
            }
        }
        for w in found.warnings {
            if !out.warnings.contains(&w) {
                out.warnings.push(w);
            }
        }
    }
    Ok(out)
}

/// `deleted_table`: leave out what goes with that table when it is deleted.
fn find(
    doc: &Document,
    object: ObjectId,
    extra: &[ExtraDax<'_>],
    deleted_table: Option<ObjectId>,
) -> Result<Dependents, CommandError> {
    if let Some(ObjectPath::Relationship { relationship }) = doc.locate(object) {
        let model = doc.model.as_ref().ok_or(CommandError::NoModel)?;
        let r = model
            .relationships
            .get(relationship)
            .ok_or(CommandError::NotFound(object))?;
        return relationship_dependents(doc, model, r, extra);
    }
    if let Some(ObjectPath::Hierarchy { table, hierarchy }) = doc.locate(object) {
        let model = doc.model.as_ref().ok_or(CommandError::NoModel)?;
        let t = model
            .tables
            .get(table)
            .ok_or(CommandError::NotFound(object))?;
        let h = t
            .hierarchies
            .get(hierarchy)
            .ok_or(CommandError::NotFound(object))?;
        return Ok(hierarchy_dependents(doc, &t.name, &h.name));
    }
    let scan = dax_scan(doc, object, extra)?;
    let mut out = Dependents {
        dependents: scan
            .references
            .into_iter()
            .filter(|r| deleted_table != Some(r.owner))
            .map(|r| Dependent {
                location: r.location,
                property: r.property,
            })
            .collect(),
        warnings: scan
            .warnings
            .into_iter()
            .filter(|(owner, _)| deleted_table != Some(*owner))
            .map(|(_, w)| w)
            .collect(),
    };
    let model = doc.model.as_ref().ok_or(CommandError::NoModel)?;
    let target = target(model, doc.locate(object), object)?;
    if deleted_table.is_none() {
        structure(model, &target, &mut out.dependents);
    }
    let probe = target.probe();
    for report in &doc.reports {
        let in_report = format!("report '{}'", report.name);
        if probe.apply_to_filters(&mut report.filters.clone()) {
            push(&mut out.dependents, &in_report, "filters");
        }
        if let Target::Table(table) = &target {
            for m in report.measures.iter().filter(|m| names_eq(&m.table, table)) {
                let location = report_measure_location(&m.table, &m.name, &report.name);
                push(&mut out.dependents, &location, "table");
            }
        }
        for page in &report.pages {
            let on_page = format!("page '{}' of {in_report}", page.display_name);
            if probe.apply_to_filters(&mut page.filters.clone()) {
                push(&mut out.dependents, &on_page, "filters");
            }
            for visual in &page.visuals {
                if probe.apply_to_visual(&mut visual.clone()) {
                    let location = format!("{} on {on_page}", visual_label(visual));
                    push(&mut out.dependents, &location, "fields");
                }
            }
        }
    }
    Ok(out)
}

/// The inspected object, by name.
enum Target {
    Table(String),
    Column { table: String, column: String },
    Measure { table: String, measure: String },
}

impl Target {
    /// A rename whose matches are exactly the references to the object.
    fn probe(&self) -> Rename {
        let new = "\u{e000}".to_owned();
        match self {
            Target::Table(old) => Rename::Table {
                old: old.clone(),
                new,
            },
            Target::Column { table, column } => Rename::Column {
                table: table.clone(),
                old: column.clone(),
                new,
            },
            Target::Measure { table, measure } => Rename::Measure {
                table: table.clone(),
                old: measure.clone(),
                new,
            },
        }
    }
}

fn target(
    model: &Model,
    path: Option<ObjectPath>,
    object: ObjectId,
) -> Result<Target, CommandError> {
    let missing = || CommandError::NotFound(object);
    let table = |t: usize| model.tables.get(t).ok_or_else(missing);
    match path.ok_or_else(missing)? {
        ObjectPath::Table { table: t } => Ok(Target::Table(table(t)?.name.clone())),
        ObjectPath::Column { table: t, column } => {
            let t = table(t)?;
            let c = t.columns.get(column).ok_or_else(missing)?;
            Ok(Target::Column {
                table: t.name.clone(),
                column: c.name.clone(),
            })
        }
        ObjectPath::Measure { table: t, measure } => {
            let t = table(t)?;
            let m = t.measures.get(measure).ok_or_else(missing)?;
            Ok(Target::Measure {
                table: t.name.clone(),
                measure: m.name.clone(),
            })
        }
        other => Err(CommandError::Unsupported {
            kind: other.kind(),
            operation: "dependents",
        }),
    }
}

/// Structured model references: relationships, hierarchies, sort-by
/// columns, role table permissions.
fn structure(model: &Model, target: &Target, out: &mut Vec<Dependent>) {
    let (table, column) = match target {
        Target::Table(table) => (table, None),
        Target::Column { table, column } => (table, Some(column.as_str())),
        Target::Measure { .. } => return,
    };
    let hits = |r: &pbi_model::ColumnRef| {
        names_eq(&r.table, table) && column.is_none_or(|c| names_eq(&r.column, c))
    };
    for r in &model.relationships {
        let location = format!(
            "relationship '{}'[{}] → '{}'[{}]",
            r.from.table, r.from.column, r.to.table, r.to.column
        );
        if hits(&r.from) {
            push(out, &location, "fromColumn");
        }
        if hits(&r.to) {
            push(out, &location, "toColumn");
        }
    }
    let Some(column) = column else {
        for role in &model.roles {
            if role
                .table_permissions
                .iter()
                .any(|p| names_eq(&p.table, table))
            {
                push(out, &format!("role '{}'", role.name), "tablePermission");
            }
        }
        return;
    };
    let Some(t) = model.table(table) else {
        return;
    };
    for h in &t.hierarchies {
        for level in h.levels.iter().filter(|l| names_eq(&l.column, column)) {
            let location = format!("level '{}'[{}].[{}]", t.name, h.name, level.name);
            push(out, &location, "column");
        }
    }
    for c in &t.columns {
        if c.sort_by_column
            .as_deref()
            .is_some_and(|s| names_eq(s, column))
            && !names_eq(&c.name, column)
        {
            let kind = match c.kind {
                ColumnKind::Calculated { .. } => "calculated column",
                _ => "column",
            };
            push(
                out,
                &format!("{kind} '{}'[{}]", t.name, c.name),
                "sortByColumn",
            );
        }
    }
}

/// The DAX that names both ends of relationship `r` in a `USERELATIONSHIP`
/// or `CROSSFILTER` call: the expressions that change or fail when it is
/// deleted. Report fields and other structure never name a relationship.
fn relationship_dependents(
    doc: &Document,
    model: &Model,
    r: &Relationship,
    extra: &[ExtraDax<'_>],
) -> Result<Dependents, CommandError> {
    let mut out = Dependents::default();
    // Without the from column no expression can name both ends.
    let Some(from) = model.column(&r.from.table, &r.from.column) else {
        return Ok(out);
    };
    let scan = dax_scan(doc, from.id, extra)?;
    for found in &scan.references {
        if pbi_dax::parse_expression(&found.text).is_ok_and(|e| names_relationship(&e, r)) {
            push(&mut out.dependents, &found.location, &found.property);
        }
    }
    let lower = |s: &str| s.to_lowercase();
    for u in &scan.unparsed {
        let text = lower(&u.text);
        let uses = text.contains("userelationship") || text.contains("crossfilter");
        if uses && text.contains(&lower(&r.from.column)) && text.contains(&lower(&r.to.column)) {
            out.warnings.push(format!(
                "{} {} does not parse and mentions both columns; it may use the relationship",
                u.location, u.property
            ));
        }
    }
    Ok(out)
}

/// Whether `expr` calls `USERELATIONSHIP` or `CROSSFILTER` on the two
/// columns of `r`, in either order.
fn names_relationship(expr: &pbi_dax::Expr, r: &Relationship) -> bool {
    let column = |e: &pbi_dax::Expr, end: &ColumnRef| match &e.kind {
        pbi_dax::ExprKind::ColumnRef {
            table: Some(table),
            column,
        } => names_eq(table, &end.table) && names_eq(column, &end.column),
        _ => false,
    };
    let mut found = false;
    pbi_dax::for_each_expr(expr, |e| {
        if let pbi_dax::ExprKind::Call { name, args, .. } = &e.kind
            && (name == "USERELATIONSHIP" || name == "CROSSFILTER")
            && let [a, b, ..] = args.as_slice()
            && ((column(a, &r.from) && column(b, &r.to))
                || (column(a, &r.to) && column(b, &r.from)))
        {
            found = true;
        }
    });
    found
}

/// Calls its argument with each field expression of a report part.
type FieldVisit<'a> = &'a dyn Fn(&mut dyn FnMut(&FieldExpr));

/// The report fields that use a level of user hierarchy `table`[`name`].
fn hierarchy_dependents(doc: &Document, table: &str, name: &str) -> Dependents {
    fn uses(field: &FieldExpr, table: &str, name: &str) -> bool {
        match field {
            FieldExpr::HierarchyLevel {
                entity,
                hierarchy,
                variation: None,
                ..
            } => names_eq(entity, table) && names_eq(hierarchy, name),
            FieldExpr::Aggregation { expr, .. } => uses(expr, table, name),
            _ => false,
        }
    }
    let hit = |visit: FieldVisit<'_>| {
        let mut found = false;
        visit(&mut |f| found |= uses(f, table, name));
        found
    };
    let mut out = Dependents::default();
    for report in &doc.reports {
        let in_report = format!("report '{}'", report.name);
        if hit(&|f| report.filters.iter().for_each(|x| x.visit_fields(f))) {
            push(&mut out.dependents, &in_report, "filters");
        }
        for page in &report.pages {
            let on_page = format!("page '{}' of {in_report}", page.display_name);
            if hit(&|f| page.filters.iter().for_each(|x| x.visit_fields(f))) {
                push(&mut out.dependents, &on_page, "filters");
            }
            for visual in &page.visuals {
                if hit(&|f| visual.visit_fields(f)) {
                    let location = format!("{} on {on_page}", visual_label(visual));
                    push(&mut out.dependents, &location, "fields");
                }
            }
        }
    }
    out
}

fn visual_label(visual: &Visual) -> String {
    let kind = if visual.visual_type.is_empty() {
        "group".to_owned()
    } else {
        visual.visual_type.clone()
    };
    format!("visual '{}' ({kind})", visual.name)
}

fn push(out: &mut Vec<Dependent>, location: &str, property: &str) {
    out.push(Dependent {
        location: location.to_owned(),
        property: property.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use pbi_model::{
        Column, ColumnRef, DataType, FieldExpr, Hierarchy, Level, Measure, Page, Projection,
        Relationship, Report, Table,
    };

    use super::*;

    fn doc() -> Document {
        let mut orders = Table::new("Orders");
        let mut month = Column::data("Month", DataType::String);
        month.sort_by_column = Some("Month No".into());
        orders.columns = vec![
            Column::data("Store ID", DataType::String),
            Column::data("Line Amount", DataType::Double),
            month,
            Column::data("Month No", DataType::Int64),
        ];
        orders.hierarchies = vec![Hierarchy {
            name: "Calendar".into(),
            levels: vec![Level {
                name: "Month".into(),
                column: "Month".into(),
                ..Level::default()
            }],
            ..Hierarchy::default()
        }];
        orders.measures = vec![
            Measure::new("Total Sales", "SUM ( Orders[Line Amount] )"),
            Measure::new("Twice", "[Total Sales] * 2"),
            Measure::new("Broken", "SUM([Total Sales]"),
            Measure::new("Unrelated", "1"),
        ];
        let mut stores = Table::new("Stores");
        stores.columns = vec![
            Column::data("Store ID", DataType::String),
            Column::calculated("Share", DataType::Double, "[Total Sales] / 2"),
        ];
        let card = |name: &str, field: FieldExpr| pbi_model::Visual {
            name: name.into(),
            visual_type: "card".into(),
            projections: vec![Projection {
                role: "Values".into(),
                field,
                query_ref: "q".into(),
                native_query_ref: None,
                display_name: None,
                active: None,
                extra: pbi_model::PropertyBag::default(),
            }],
            ..pbi_model::Visual::default()
        };
        let measure = |name: &str| FieldExpr::Measure {
            entity: "Orders".into(),
            property: name.into(),
        };
        let mut doc = Document {
            model: Some(Model {
                tables: vec![orders, stores],
                relationships: vec![Relationship {
                    from: ColumnRef::new("Orders", "Store ID"),
                    to: ColumnRef::new("Stores", "Store ID"),
                    ..Relationship::default()
                }],
                ..Model::default()
            }),
            reports: vec![Report {
                name: "Sales".into(),
                pages: vec![Page {
                    display_name: "Overview".into(),
                    visuals: vec![
                        card("card1", measure("Total Sales")),
                        card("card2", measure("Unrelated")),
                    ],
                    ..Page::default()
                }],
                ..Report::default()
            }],
            ..Document::default()
        };
        doc.assign_ids();
        doc
    }

    fn locations(found: &Dependents) -> Vec<(&str, &str)> {
        found
            .dependents
            .iter()
            .map(|d| (d.location.as_str(), d.property.as_str()))
            .collect()
    }

    #[test]
    fn measure_dependents_cover_dax_and_report_fields() {
        let d = doc();
        let total = d
            .model
            .as_ref()
            .unwrap()
            .measure("Orders", "Total Sales")
            .unwrap()
            .id;
        let found = object_dependents(&d, total).unwrap();
        assert_eq!(
            locations(&found),
            [
                ("measure 'Orders'[Twice]", "expression"),
                ("column 'Stores'[Share]", "expression"),
                (
                    "visual 'card1' (card) on page 'Overview' of report 'Sales'",
                    "fields"
                ),
            ]
        );
        assert_eq!(found.warnings.len(), 1, "{found:#?}");
        assert!(found.warnings[0].contains("'Orders'[Broken]"));
        assert!(found.warnings[0].contains("may refer to it"));

        let unrelated = d
            .model
            .as_ref()
            .unwrap()
            .measure("Orders", "Twice")
            .unwrap()
            .id;
        assert_eq!(
            object_dependents(&d, unrelated).unwrap(),
            Dependents::default()
        );
    }

    #[test]
    fn column_and_table_dependents_include_model_structure() {
        let d = doc();
        let model = d.model.as_ref().unwrap();
        let month = model.column("Orders", "Month No").unwrap().id;
        let found = object_dependents(&d, month).unwrap();
        assert_eq!(
            locations(&found),
            [("column 'Orders'[Month]", "sortByColumn")]
        );
        let month = model.column("Orders", "Month").unwrap().id;
        assert_eq!(
            locations(&object_dependents(&d, month).unwrap()),
            [("level 'Orders'[Calendar].[Month]", "column")]
        );
        let stores = model.table("Stores").unwrap().id;
        assert_eq!(
            locations(&object_dependents(&d, stores).unwrap()),
            [(
                "relationship 'Orders'[Store ID] → 'Stores'[Store ID]",
                "toColumn"
            )]
        );
        let page = d.reports[0].pages[0].id;
        assert!(matches!(
            object_dependents(&d, page),
            Err(CommandError::Unsupported {
                operation: "dependents",
                ..
            })
        ));
    }

    #[test]
    fn delete_dependents_leave_out_what_goes_with_a_table() {
        let d = doc();
        let model = d.model.as_ref().unwrap();
        let orders = model.table("Orders").unwrap().id;
        // Orders' own measures and its relationship go with it; Stores[Share]
        // uses its measure [Total Sales] without naming the table.
        let found = delete_dependents(&d, orders).unwrap();
        assert_eq!(
            locations(&found),
            [
                (
                    "visual 'card1' (card) on page 'Overview' of report 'Sales'",
                    "fields"
                ),
                (
                    "visual 'card2' (card) on page 'Overview' of report 'Sales'",
                    "fields"
                ),
                ("column 'Stores'[Share]", "expression"),
            ]
        );
        assert_eq!(
            found.warnings,
            Vec::<String>::new(),
            "Orders[Broken] goes too"
        );
        // Columns and measures: the same as object_dependents.
        let month = model.column("Orders", "Month").unwrap().id;
        assert_eq!(
            delete_dependents(&d, month).unwrap(),
            object_dependents(&d, month).unwrap()
        );
    }
    #[test]
    fn report_level_measures_are_dependents() {
        let mut d = doc();
        d.reports[0].measures.push(pbi_model::ReportMeasure {
            table: "Orders".into(),
            name: "Per Line".into(),
            expression: "DIVIDE([Total Sales], COUNTROWS(Orders))".into(),
            ..pbi_model::ReportMeasure::default()
        });
        let model = d.model.as_ref().unwrap();
        let location = "report-level measure 'Orders'[Per Line] of report 'Sales'";
        let total = model.measure("Orders", "Total Sales").unwrap().id;
        assert!(
            locations(&object_dependents(&d, total).unwrap()).contains(&(location, "expression"))
        );
        // It stays in the report when its table is deleted.
        let orders = model.table("Orders").unwrap().id;
        let found = delete_dependents(&d, orders).unwrap();
        let found = locations(&found);
        assert!(found.contains(&(location, "expression")), "{found:?}");
        assert!(found.contains(&(location, "table")), "{found:?}");
        let stores = model.table("Stores").unwrap().id;
        let found = object_dependents(&d, stores).unwrap();
        assert!(!locations(&found).iter().any(|(l, _)| *l == location));
    }

    #[test]
    fn relationship_dependents_are_the_dax_that_names_both_ends() {
        let mut d = doc();
        let model = d.model.as_mut().unwrap();
        model.tables[0].measures.extend([
            Measure::new(
                "Via",
                "CALCULATE ( [Total Sales], USERELATIONSHIP ( Stores[Store ID], 'Orders'[store id] ) )",
            ),
            Measure::new(
                "Both",
                "CALCULATE ( [Total Sales], CROSSFILTER ( Orders[Store ID], Stores[Store ID], BOTH ) )",
            ),
            // Names both columns but not in a relationship function.
            Measure::new(
                "Count",
                "COUNTROWS ( Orders ) + DISTINCTCOUNT ( Stores[Store ID] ) + MAX ( Orders[Store ID] )",
            ),
            Measure::new(
                "Other",
                "CALCULATE ( 1, USERELATIONSHIP ( Orders[Month No], Stores[Store ID] ) )",
            ),
            Measure::new(
                "Typo",
                "CALCULATE ( 1, USERELATIONSHIP ( Orders[Store ID], Stores[Store ID] )",
            ),
        ]);
        d.assign_ids();
        let relationship = d.model.as_ref().unwrap().relationships[0].id;
        let found = object_dependents(&d, relationship).unwrap();
        assert_eq!(
            locations(&found),
            [
                ("measure 'Orders'[Via]", "expression"),
                ("measure 'Orders'[Both]", "expression"),
            ]
        );
        assert_eq!(found.warnings.len(), 1, "{found:#?}");
        assert!(found.warnings[0].starts_with("measure 'Orders'[Typo] expression does not parse"));

        // A relationship whose column is gone has no DAX dependents.
        d.model.as_mut().unwrap().relationships[0].from.column = "Gone".into();
        assert_eq!(
            object_dependents(&d, relationship).unwrap(),
            Dependents::default()
        );
    }

    #[test]
    fn hierarchy_dependents_are_the_report_fields_on_its_levels() {
        let mut d = doc();
        let level = |hierarchy: &str| FieldExpr::HierarchyLevel {
            entity: "orders".into(),
            hierarchy: hierarchy.into(),
            level: "Month".into(),
            variation: None,
        };
        let page = &mut d.reports[0].pages[0];
        page.visuals[0].projections[0].field = level("calendar");
        page.visuals[1].projections[0].field = level("Other");
        let calendar = d.model.as_ref().unwrap().tables[0].hierarchies[0].id;
        assert_eq!(
            locations(&object_dependents(&d, calendar).unwrap()),
            [(
                "visual 'card1' (card) on page 'Overview' of report 'Sales'",
                "fields"
            )]
        );
        assert_eq!(
            delete_dependents(&d, calendar).unwrap(),
            object_dependents(&d, calendar).unwrap()
        );
    }
}
