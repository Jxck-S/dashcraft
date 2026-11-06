//! What refers to a table, column or measure.
//!
//! [`object_dependents`] lists the places that would break if the object
//! were deleted: DAX expressions (measures, dynamic format strings,
//! calculated columns and tables, calculation items, role filters), model
//! structure (relationship ends, hierarchy levels, sort-by columns, role
//! table permissions) and report fields (visual projections, sorts, filters
//! and formatting, page and report filters).
//! [`Session::object_dependents`] adds the session's derived DAX columns.
//! References that cannot be confirmed are reported as warnings.

use pbi_model::{
    ColumnKind, CommandError, Document, Model, ObjectId, ObjectPath, Rename, Visual, names_eq,
};
use serde::{Deserialize, Serialize};

use crate::DerivedKind;
use crate::edit::{ExtraDax, dax_references};
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

/// What in `doc` refers to table, column or measure `object` (see the
/// [module docs](self)).
///
/// # Errors
/// `object` is unknown or not a table, column or measure, or there is no
/// model.
pub fn object_dependents(doc: &Document, object: ObjectId) -> Result<Dependents, CommandError> {
    find(doc, object, &[])
}

impl Session {
    /// [`object_dependents`] on the current document, including the DAX of
    /// the session's derived columns.
    ///
    /// # Errors
    /// See [`object_dependents`].
    pub fn object_dependents(&self, object: ObjectId) -> Result<Dependents, CommandError> {
        let derived = self.derived_columns();
        let extra: Vec<ExtraDax<'_>> = derived
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
            .collect();
        find(&self.document(), object, &extra)
    }
}

fn find(
    doc: &Document,
    object: ObjectId,
    extra: &[ExtraDax<'_>],
) -> Result<Dependents, CommandError> {
    let mut out = dax_references(doc, object, extra)?;
    let model = doc.model.as_ref().ok_or(CommandError::NoModel)?;
    let target = target(model, doc.locate(object), object)?;
    structure(model, &target, &mut out.dependents);
    let probe = target.probe();
    for report in &doc.reports {
        let in_report = format!("report '{}'", report.name);
        if probe.apply_to_filters(&mut report.filters.clone()) {
            push(&mut out.dependents, &in_report, "filters");
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
}
