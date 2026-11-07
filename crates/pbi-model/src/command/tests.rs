use pretty_assertions::assert_eq;

use super::*;
use crate::query::{Condition, FieldExpr};
use crate::report::Position;
use crate::semantic::{
    Column, ColumnKind, ColumnRef, DataType, Measure, Relationship, SummarizeBy,
};
use crate::test_support::sample_document;

fn table_id(doc: &Document, name: &str) -> ObjectId {
    doc.model.as_ref().unwrap().table(name).unwrap().id
}

fn column_id(doc: &Document, table: &str, column: &str) -> ObjectId {
    doc.model
        .as_ref()
        .unwrap()
        .column(table, column)
        .unwrap()
        .id
}

fn measure_id(doc: &Document, table: &str, measure: &str) -> ObjectId {
    doc.model
        .as_ref()
        .unwrap()
        .measure(table, measure)
        .unwrap()
        .id
}

/// execute → check → undo == original → redo == after.
fn assert_round_trip(
    doc: &mut Document,
    command: impl Command + 'static,
    check: impl FnOnce(&Document, &[ChangeEvent]),
) {
    let original = doc.clone();
    let mut history = History::new();
    let events = history.execute(doc, command).unwrap();
    check(doc, &events);
    let after = doc.clone();
    assert_ne!(after, original);
    history.undo(doc).unwrap();
    assert_eq!(*doc, original);
    history.redo(doc).unwrap();
    assert_eq!(*doc, after);
    history.undo(doc).unwrap();
    assert_eq!(*doc, original);
}

#[test]
fn rename_table_rewrites_references_and_undoes_exactly() {
    let mut doc = sample_document();
    let id = table_id(&doc, "Stores");
    let rel_id = doc.model.as_ref().unwrap().relationships[0].id;
    let command = RenameTable {
        table: id,
        new_name: "Shops".into(),
    };
    assert_round_trip(&mut doc, command, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert!(model.table("Stores").is_none());
        assert_eq!(model.table_by_id(id).unwrap().name, "Shops");
        assert_eq!(model.relationships[0].to.table, "Shops");
        assert_eq!(model.relationships[0].from.table, "Orders");
        assert_eq!(model.roles[0].table_permissions[0].table, "Shops");

        let report = &doc.reports[0];
        let filter = &report.filters[0];
        assert_eq!(filter.field.as_ref().unwrap().entity(), Some("Shops"));
        let Some(Condition::In { exprs, .. }) = &filter.condition else {
            panic!("expected In condition");
        };
        assert_eq!(exprs[0].entity(), Some("Shops"));
        assert_eq!(
            filter.raw["field"]["Column"]["Expression"]["SourceRef"]["Entity"],
            "Shops"
        );
        assert_eq!(filter.raw["filter"]["From"][0]["Entity"], "Shops");

        let page = &report.pages[0];
        let slicer = &page.visuals[0];
        assert_eq!(slicer.projections[0].query_ref, "Shops.Store Name");
        assert_eq!(
            slicer.projections[0].native_query_ref.as_deref(),
            Some("Store Name")
        );
        let bar = &page.visuals[2];
        assert_eq!(
            bar.projections[1].query_ref,
            "CountNonNull(Shops.Store Name)"
        );
        assert_eq!(bar.sort[0].field.entity(), Some("Shops"));
        assert_eq!(
            bar.objects["dataPoint"][0]["selector"]["metadata"],
            "Shops.Region"
        );
        // Untouched references stay as they were.
        assert_eq!(
            page.filters[0].field.as_ref().unwrap().entity(),
            Some("Orders")
        );

        assert!(events.contains(&ChangeEvent::ObjectRenamed {
            id,
            old: "Stores".into(),
            new: "Shops".into(),
        }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: rel_id }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: bar.id }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: report.id }));
        // The unrelated table visual on page 2 is not reported.
        let other = &report.pages[1].visuals[0];
        assert!(!events.iter().any(|e| e.id() == other.id));
    });
}

#[test]
fn rename_column_updates_model_and_report_references() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut history = History::new();

    let rank = column_id(&doc, "Stores", "Region Rank");
    history
        .execute(
            &mut doc,
            RenameColumn {
                column: rank,
                new_name: "Rank".into(),
            },
        )
        .unwrap();
    let region = column_id(&doc, "Stores", "Region");
    let events = history
        .execute(
            &mut doc,
            RenameColumn {
                column: region,
                new_name: "Area".into(),
            },
        )
        .unwrap();
    let key = column_id(&doc, "Orders", "Store ID");
    history
        .execute(
            &mut doc,
            RenameColumn {
                column: key,
                new_name: "Key".into(),
            },
        )
        .unwrap();

    let model = doc.model.as_ref().unwrap();
    let stores_table = model.table("Stores").unwrap();
    let area = stores_table.column("Area").unwrap();
    assert_eq!(area.id, region);
    assert_eq!(area.sort_by_column.as_deref(), Some("Rank"));
    // The source column is the M output name and must not change.
    assert_eq!(area.source_column(), Some("Region"));
    assert_eq!(stores_table.hierarchies[0].levels[0].column, "Area");
    assert_eq!(model.relationships[0].from.column, "Key");
    assert_eq!(model.relationships[0].to.column, "Store ID");

    let report = &doc.reports[0];
    let filter = &report.filters[0];
    assert_eq!(filter.field.as_ref().unwrap().property(), Some("Area"));
    // Alias-based reference (`SourceRef.Source = "s"`) in the raw JSON.
    assert_eq!(
        filter.raw["filter"]["Where"][0]["Condition"]["In"]["Expressions"][0]["Column"]["Property"],
        "Area"
    );
    let bar = &report.pages[0].visuals[2];
    assert_eq!(bar.projections[0].query_ref, "Stores.Area");
    assert_eq!(bar.projections[0].native_query_ref.as_deref(), Some("Area"));
    assert_eq!(
        bar.filters[0].field.as_ref().unwrap().property(),
        Some("Area")
    );
    assert_eq!(
        bar.objects["dataPoint"][0]["selector"]["metadata"],
        "Stores.Area"
    );
    assert!(events.contains(&ChangeEvent::ObjectRenamed {
        id: region,
        old: "Region".into(),
        new: "Area".into(),
    }));

    let undo_events = history.undo(&mut doc).unwrap();
    assert!(undo_events.contains(&ChangeEvent::ObjectRenamed {
        id: key,
        old: "Key".into(),
        new: "Store ID".into(),
    }));
    history.undo(&mut doc).unwrap();
    history.undo(&mut doc).unwrap();
    assert_eq!(doc, original);
    assert_eq!(history.undo(&mut doc), Err(CommandError::NothingToUndo));
}

#[test]
fn rename_measure_updates_projections() {
    let mut doc = sample_document();
    let id = measure_id(&doc, "Stores", "Active Stores");
    let command = RenameMeasure {
        measure: id,
        new_name: "Stores With Sales".into(),
    };
    assert_round_trip(&mut doc, command, |doc, events| {
        let card = &doc.reports[0].pages[0].visuals[1];
        assert_eq!(
            card.projections[0].field,
            FieldExpr::measure("Stores", "Stores With Sales")
        );
        assert_eq!(card.projections[0].query_ref, "Stores.Stores With Sales");
        assert_eq!(
            card.projections[0].native_query_ref.as_deref(),
            Some("Stores With Sales")
        );
        assert_eq!(events.len(), 2, "{events:?}");
    });
}

#[test]
fn rename_case_only_and_same_name() {
    let mut doc = sample_document();
    let id = table_id(&doc, "Orders");
    let mut history = History::new();
    history
        .execute(
            &mut doc,
            RenameTable {
                table: id,
                new_name: "ORDERS".into(),
            },
        )
        .unwrap();
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.relationships[0].from.table, "ORDERS");

    let before = doc.clone();
    let events = history
        .execute(
            &mut doc,
            RenameTable {
                table: id,
                new_name: "ORDERS".into(),
            },
        )
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(doc, before);
}

fn with_query_order(doc: &mut Document) {
    doc.model.as_mut().unwrap().annotations = vec![
        crate::props::Annotation::new(
            "PBI_QueryOrder",
            r#"["StorageContainerUrl","Stores","Orders"]"#,
        ),
        crate::props::Annotation::new("PBI_ProTooling", r#"["DevMode"]"#),
    ];
}

fn query_order(doc: &Document) -> &str {
    &doc.model.as_ref().unwrap().annotations[0].value
}

#[test]
fn rename_expression_updates_the_query_order_and_undoes_exactly() {
    let mut doc = sample_document();
    with_query_order(&mut doc);
    let entity = |source: &str| crate::semantic::PartitionSource::Entity {
        entity_name: "orders".into(),
        expression_source: Some(source.into()),
        schema_name: None,
    };
    doc.model.as_mut().unwrap().tables[1].partitions[0].source = entity("storagecontainerurl");
    let model = doc.model.as_ref().unwrap();
    let (id, model_id) = (model.expressions[0].id, model.id);
    let command = RenameExpression {
        expression: id,
        new_name: "Storage URL".into(),
    };
    assert_round_trip(&mut doc, command, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert_eq!(model.expressions[0].name, "Storage URL");
        assert_eq!(
            model.expressions[0].description.as_deref(),
            Some("Azure Blob container holding the source CSVs.")
        );
        assert_eq!(query_order(doc), r#"["Storage URL","Stores","Orders"]"#);
        assert_eq!(model.annotations[1].value, r#"["DevMode"]"#);
        assert_eq!(model.tables[1].partitions[0].source, entity("Storage URL"));
        assert!(events.contains(&ChangeEvent::ObjectRenamed {
            id,
            old: "StorageContainerUrl".into(),
            new: "Storage URL".into(),
        }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: model_id }));
    });

    // A table rename renames the query too; a model without the annotation
    // or with an unreadable one is left as it is.
    let stores = table_id(&doc, "Stores");
    let mut history = History::new();
    history
        .execute(
            &mut doc,
            RenameTable {
                table: stores,
                new_name: "Shops".into(),
            },
        )
        .unwrap();
    assert_eq!(
        query_order(&doc),
        r#"["StorageContainerUrl","Shops","Orders"]"#
    );
    doc.model.as_mut().unwrap().annotations[0].value = "not json".into();
    let before = doc.model.as_ref().unwrap().annotations.clone();
    history
        .execute(
            &mut doc,
            RenameTable {
                table: stores,
                new_name: "Outlets".into(),
            },
        )
        .unwrap();
    assert_eq!(doc.model.as_ref().unwrap().annotations, before);
}

#[test]
fn queries_share_one_namespace() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut history = History::new();
    let parameter = doc.model.as_ref().unwrap().expressions[0].id;
    let stores = table_id(&doc, "Stores");
    let calculated = table_id(&doc, "Unknown Store Orders");
    let conflicts: Vec<Box<dyn Command>> = vec![
        Box::new(RenameExpression {
            expression: parameter,
            new_name: "orders".into(),
        }),
        Box::new(RenameTable {
            table: stores,
            new_name: "storagecontainerurl".into(),
        }),
    ];
    for command in conflicts {
        let err = history.execute_boxed(&mut doc, command).unwrap_err();
        assert!(matches!(err, CommandError::NameConflict { .. }), "{err}");
        assert_eq!(doc, original);
    }
    let err = history
        .execute(
            &mut doc,
            RenameExpression {
                expression: stores,
                new_name: "X".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(err, CommandError::WrongKind { .. }), "{err}");
    // A calculated table is not a query: it may share a name with one.
    history
        .execute(
            &mut doc,
            RenameTable {
                table: calculated,
                new_name: "StorageContainerUrl".into(),
            },
        )
        .unwrap();
    history
        .execute(
            &mut doc,
            RenameExpression {
                expression: parameter,
                new_name: "STORAGECONTAINERURL".into(),
            },
        )
        .unwrap();
    assert_eq!(
        doc.model.as_ref().unwrap().expressions[0].name,
        "STORAGECONTAINERURL"
    );
}

type ErrorCheck = fn(&CommandError) -> bool;

#[test]
fn invalid_renames_fail_without_changing_the_document() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut history = History::new();
    let stores = table_id(&doc, "Stores");
    let total = measure_id(&doc, "Stores", "Store Count");
    let key = column_id(&doc, "Stores", "Store ID");

    let conflict: ErrorCheck = |e| matches!(e, CommandError::NameConflict { .. });
    let cases: Vec<(Box<dyn Command>, ErrorCheck)> = vec![
        (
            Box::new(RenameTable {
                table: stores,
                new_name: "orders".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameTable {
                table: stores,
                new_name: "  ".into(),
            }),
            |e| matches!(e, CommandError::InvalidName(_)),
        ),
        (
            Box::new(RenameTable {
                table: key,
                new_name: "X".into(),
            }),
            |e| {
                matches!(
                    e,
                    CommandError::WrongKind {
                        expected: ObjectKind::Table,
                        actual: ObjectKind::Column,
                        ..
                    }
                )
            },
        ),
        (
            Box::new(RenameMeasure {
                measure: total,
                new_name: "ACTIVE STORES".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameMeasure {
                measure: total,
                new_name: "store id".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameColumn {
                column: key,
                new_name: "Store Name".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameColumn {
                column: key,
                new_name: "Active Stores".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameColumn {
                column: ObjectId(999_999),
                new_name: "X".into(),
            }),
            |e| matches!(e, CommandError::NotFound(_)),
        ),
    ];
    let revision = history.revision();
    for (command, expected) in cases {
        let label = command.label();
        let err = history.execute_boxed(&mut doc, command).unwrap_err();
        assert!(expected(&err), "{label}: unexpected error {err}");
        assert_eq!(doc, original);
        assert_eq!(history.revision(), revision);
    }
    assert!(!history.can_undo());
    assert!(!history.is_dirty());
}

#[test]
fn property_edits_round_trip() {
    let mut doc = sample_document();
    let active = measure_id(&doc, "Stores", "Active Stores");
    assert_round_trip(
        &mut doc,
        SetMeasureExpression {
            measure: active,
            expression: "COUNTROWS(FILTER(Stores, [Order Count] > 0))".into(),
        },
        |doc, events| {
            let m = doc
                .model
                .as_ref()
                .unwrap()
                .measure("Stores", "Active Stores");
            assert!(m.unwrap().expression.starts_with("COUNTROWS(FILTER"));
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: active }]);
        },
    );

    let region = column_id(&doc, "Stores", "Region");
    assert_round_trip(
        &mut doc,
        SetDescription {
            id: region,
            text: None,
        },
        |doc, _| {
            let c = doc.model.as_ref().unwrap().column("Stores", "Region");
            assert_eq!(c.unwrap().description, None);
        },
    );
    let blob = doc.model.as_ref().unwrap().expressions[0].id;
    assert_round_trip(
        &mut doc,
        SetDescription {
            id: blob,
            text: Some("Container URL".into()),
        },
        |doc, _| {
            assert_eq!(doc.find(blob).unwrap().description(), Some("Container URL"));
        },
    );

    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let moved = Position {
        x: 20.0,
        y: 30.0,
        z: 5.0,
        width: 300.0,
        height: 80.0,
        tab_order: Some(1),
        angle: None,
    };
    assert_round_trip(
        &mut doc,
        SetVisualPosition {
            visual: slicer,
            position: moved.clone(),
        },
        |doc, events| {
            assert_eq!(doc.reports[0].pages[0].visuals[0].position, moved);
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: slicer }]);
        },
    );

    let err = SetDescription {
        id: slicer,
        text: Some("x".into()),
    }
    .apply(&mut doc)
    .unwrap_err();
    assert_eq!(
        err,
        CommandError::Unsupported {
            kind: ObjectKind::Visual,
            operation: "descriptions",
        }
    );
}

#[test]
fn history_tracks_dirty_state_and_labels() {
    let mut doc = sample_document();
    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let move_to = |x: f64| SetVisualPosition {
        visual: slicer,
        position: Position {
            x,
            ..Position::default()
        },
    };
    let mut history = History::new();
    assert!(!history.is_dirty());
    history.execute(&mut doc, move_to(1.0)).unwrap();
    assert!(history.is_dirty());
    assert_eq!(history.undo_label(), Some("Move visual"));
    history.mark_saved();
    assert!(!history.is_dirty());
    let saved_revision = history.revision();

    history.undo(&mut doc).unwrap();
    assert!(history.revision() > saved_revision);
    assert!(history.is_dirty());
    assert_eq!(history.redo_label(), Some("Move visual"));
    let undo_revision = history.revision();
    history.redo(&mut doc).unwrap();
    assert!(history.revision() > undo_revision);
    assert!(!history.is_dirty());

    history.execute(&mut doc, move_to(2.0)).unwrap();
    assert!(history.is_dirty());
    history.undo(&mut doc).unwrap();
    assert!(!history.is_dirty());

    // Branching after an undo drops the redo stack; the saved state is gone.
    history.undo(&mut doc).unwrap();
    history.execute(&mut doc, move_to(2.0)).unwrap();
    assert!(!history.can_redo());
    assert!(history.is_dirty());
    assert_eq!(history.redo(&mut doc), Err(CommandError::NothingToRedo));

    // Clearing keeps the dirty flag.
    history.clear();
    assert!(history.is_dirty());
    assert!(!history.can_undo());
    history.mark_saved();
    assert!(!history.is_dirty());
}

#[test]
fn history_no_op_does_not_advance_revision_or_clear_redo() {
    let mut doc = sample_document();
    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let original_position = doc.reports[0].pages[0].visuals[0].position.clone();
    let move_to = |position| SetVisualPosition {
        visual: slicer,
        position,
    };
    let mut history = History::new();

    let result = history
        .execute_with_status(
            &mut doc,
            move_to(Position {
                x: 10.0,
                ..original_position.clone()
            }),
        )
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.revision, history.revision());
    let changed_revision = history.revision();
    let result = history.undo_with_status(&mut doc).unwrap();
    assert!(result.changed);
    assert_eq!(result.revision, history.revision());
    let undone_revision = history.revision();
    assert!(undone_revision > changed_revision);

    let result = history
        .execute_with_status(&mut doc, move_to(original_position))
        .unwrap();
    assert!(!result.changed);
    assert!(result.events.is_empty());
    assert_eq!(result.revision, undone_revision);
    assert_eq!(history.revision(), undone_revision);
    assert!(history.can_redo());
}

#[test]
fn history_checks_can_refuse_edits_undo_and_redo() {
    let mut doc = sample_document();
    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let move_to = |x: f64| -> Box<dyn Command> {
        Box::new(SetVisualPosition {
            visual: slicer,
            position: Position {
                x,
                ..Position::default()
            },
        })
    };
    let refuse =
        |_: &Document| -> Result<(), CommandError> { Err(CommandError::Rejected("no".to_owned())) };
    let original = doc.clone();
    let mut history = History::new();

    let err = history
        .execute_boxed_checked(&mut doc, move_to(5.0), refuse)
        .unwrap_err();
    assert_eq!(err, CommandError::Rejected("no".to_owned()));
    assert_eq!(doc, original);
    assert_eq!(history.revision(), 0);
    assert!(!history.can_undo());

    let (result, seen) = history
        .execute_boxed_checked(&mut doc, move_to(5.0), |next| {
            Ok(next.reports[0].pages[0].visuals[0].position.x)
        })
        .unwrap();
    assert!(result.changed);
    assert_eq!(seen, Some(5.0));
    let edited = doc.clone();

    assert!(history.undo_checked(&mut doc, refuse).is_err());
    assert_eq!(doc, edited);
    assert!(history.can_undo());
    assert_eq!(history.revision(), 1);
    history.undo_checked(&mut doc, |_| Ok(())).unwrap();
    assert_eq!(doc, original);

    assert!(history.redo_checked(&mut doc, refuse).is_err());
    assert_eq!(doc, original);
    assert!(history.can_redo());
    let (result, _) = history.redo_checked(&mut doc, |_| Ok(())).unwrap();
    assert!(result.changed);
    assert_eq!(doc, edited);

    // A no-op skips the check.
    let (result, seen) = history
        .execute_boxed_checked(&mut doc, move_to(5.0), refuse)
        .unwrap();
    assert!(!result.changed);
    assert_eq!(seen, None);
}

#[test]
fn history_limit_keeps_dirty_tracking_correct() {
    let mut doc = sample_document();
    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let mut history = History::with_limit(2);
    for x in [1.0, 2.0, 3.0] {
        history
            .execute(
                &mut doc,
                SetVisualPosition {
                    visual: slicer,
                    position: Position {
                        x,
                        ..Position::default()
                    },
                },
            )
            .unwrap();
    }
    assert_eq!(history.undo_depth(), 2);
    history.undo(&mut doc).unwrap();
    history.undo(&mut doc).unwrap();
    assert!(!history.can_undo());
    // The first move can no longer be undone, so this is not the saved state.
    assert!(history.is_dirty());
    assert_eq!(doc.reports[0].pages[0].visuals[0].position.x, 1.0);
}

#[test]
fn command_group_is_atomic() {
    let mut doc = sample_document();
    let original = doc.clone();
    let active = measure_id(&doc, "Stores", "Active Stores");
    let stores = table_id(&doc, "Stores");

    let failing = CommandGroup::new("Edit")
        .with(SetMeasureExpression {
            measure: active,
            expression: "1".into(),
        })
        .with(RenameTable {
            table: stores,
            new_name: "Orders".into(),
        });
    let mut history = History::new();
    assert!(matches!(
        history.execute(&mut doc, failing),
        Err(CommandError::NameConflict { .. })
    ));
    assert_eq!(doc, original);
    assert_eq!(history.revision(), 0);

    let group = CommandGroup::new("Edit and rename")
        .with(SetMeasureExpression {
            measure: active,
            expression: "1".into(),
        })
        .with(RenameTable {
            table: stores,
            new_name: "Shops".into(),
        });
    let events = history.execute(&mut doc, group).unwrap();
    assert!(events.iter().any(|e| e.id() == active));
    assert!(events.iter().any(|e| e.id() == stores));
    assert_eq!(history.undo_label(), Some("Edit and rename"));
    let after = doc.clone();
    history.undo(&mut doc).unwrap();
    assert_eq!(doc, original);
    history.redo(&mut doc).unwrap();
    assert_eq!(doc, after);
}

/// Documents equal except for the ID generator, which undo never rewinds.
fn assert_same_content(actual: &Document, expected: &Document) {
    let mut actual = actual.clone();
    actual.ids = expected.ids.clone();
    assert_eq!(actual, *expected);
}

/// The sample document with the card and the bar in a group placed around
/// them (the card stored relative to the group, the bar in page units).
fn grouped_document() -> (Document, [ObjectId; 3]) {
    use crate::report::{Visual, VisualKind};
    let mut doc = sample_document();
    let page = &mut doc.reports[0].pages[0];
    page.visuals[1].parent_group_name = Some("grp".into());
    page.visuals[1].position.x = 0.0;
    page.visuals[1].position.y = 0.0;
    page.visuals[2].parent_group_name = Some("grp".into());
    page.visuals.push(Visual {
        name: "grp".into(),
        kind: VisualKind::Group,
        position: Position {
            x: 240.0,
            y: 10.0,
            z: 500.0,
            width: 620.0,
            height: 300.0,
            ..Position::default()
        },
        ..Visual::default()
    });
    doc.assign_ids();
    let ids = [1, 2, 3].map(|i| doc.reports[0].pages[0].visuals[i].id);
    (doc, ids)
}

#[test]
fn move_visual_takes_a_groups_members_along_and_undoes_exactly() {
    let (mut doc, [card, bar, grp]) = grouped_document();
    let resize = MoveVisual {
        visual: grp,
        x: Some(40.0),
        y: None,
        width: Some(310.0),
        height: None,
    };
    assert_eq!(resize.label(), "Resize visual");
    assert_round_trip(&mut doc, resize, |doc, events| {
        let v = &doc.reports[0].pages[0].visuals;
        let at = |i: usize| {
            let p = &v[i].position;
            [p.x, p.y, p.width, p.height]
        };
        assert_eq!(at(3), [40.0, 10.0, 310.0, 300.0]);
        // Relative card: offsets and size scale by 0.5 horizontally.
        assert_eq!(at(1), [0.0, 0.0, 100.0, 100.0]);
        // Page-unit bar: 220 from the group's left edge becomes 110.
        assert_eq!(at(2), [150.0, 10.0, 200.0, 300.0]);
        assert_eq!(
            events,
            [grp, card, bar].map(|id| ChangeEvent::ObjectChanged { id })
        );
    });

    let original = doc.clone();
    for bad in [
        MoveVisual {
            visual: grp,
            x: Some(f64::NAN),
            y: None,
            width: None,
            height: None,
        },
        MoveVisual {
            visual: grp,
            x: None,
            y: None,
            width: Some(0.0),
            height: None,
        },
        MoveVisual {
            visual: grp,
            x: None,
            y: None,
            width: None,
            height: Some(f64::INFINITY),
        },
    ] {
        assert!(matches!(
            bad.apply(&mut doc),
            Err(CommandError::Rejected(_))
        ));
        assert_eq!(doc, original);
    }
    assert!(matches!(
        MoveVisual {
            visual: measure_id(&doc, "Stores", "Active Stores"),
            x: Some(1.0),
            y: None,
            width: None,
            height: None,
        }
        .apply(&mut doc),
        Err(CommandError::WrongKind { .. })
    ));
}

#[test]
fn batched_moves_compose() {
    let (mut doc, [_, _, grp]) = grouped_document();
    let step = |x, width| MoveVisual {
        visual: grp,
        x,
        y: None,
        width,
        height: None,
    };
    let mut history = History::new();
    history
        .execute(
            &mut doc,
            CommandGroup::new("Move and resize")
                .with(step(Some(300.0), None))
                .with(step(None, Some(1240.0))),
        )
        .unwrap();
    let p = &doc.reports[0].pages[0].visuals[3].position;
    assert_eq!((p.x, p.width), (300.0, 1240.0));
    // Bar: 220 from the group's edge, doubled, after the move.
    assert_eq!(doc.reports[0].pages[0].visuals[2].position.x, 740.0);
}

#[test]
fn create_measure_round_trips_and_never_reuses_ids() {
    let mut doc = sample_document();
    let original = doc.clone();
    let orders = table_id(&doc, "Orders");
    let mut measure = Measure::new("Order Count", "COUNTROWS(Orders)");
    measure.format_string = Some("#,0".into());
    measure.id = ObjectId(424_242); // ignored: creates always get a fresh ID
    let id = CreateMeasure::created_id(&doc);
    let mut history = History::new();
    let events = history
        .execute(
            &mut doc,
            CreateMeasure {
                table: orders,
                measure,
                index: None,
            },
        )
        .unwrap();
    assert_eq!(events, [ChangeEvent::ObjectAdded { id }]);
    assert_eq!(history.undo_label(), Some("Create measure 'Order Count'"));
    let created = doc
        .model
        .as_ref()
        .unwrap()
        .measure("Orders", "Order Count")
        .unwrap();
    assert_eq!(
        (created.id, created.format_string.as_deref()),
        (id, Some("#,0"))
    );
    assert!(matches!(doc.locate(id), Some(ObjectPath::Measure { .. })));
    let after = doc.clone();

    let events = history.undo(&mut doc).unwrap();
    assert_eq!(events, [ChangeEvent::ObjectRemoved { id }]);
    assert_same_content(&doc, &original);
    assert!(doc.locate(id).is_none());
    assert!(!history.is_dirty());
    history.redo(&mut doc).unwrap();
    assert_eq!(doc, after, "redo restores the measure with the same ID");

    history.undo(&mut doc).unwrap();
    let stores = table_id(&doc, "Stores");
    let next = CreateMeasure::created_id(&doc);
    assert!(next > id, "IDs are never reused after undo");
    history
        .execute(
            &mut doc,
            CreateMeasure {
                table: stores,
                measure: Measure::new("First", "1"),
                index: Some(0),
            },
        )
        .unwrap();
    let measures = &doc.model.as_ref().unwrap().tables[0].measures;
    assert_eq!((measures[0].name.as_str(), measures[0].id), ("First", next));
    assert!(!history.can_redo());
}

#[test]
fn invalid_creates_fail_without_changing_the_document() {
    let mut doc = sample_document();
    let original = doc.clone();
    let stores = table_id(&doc, "Stores");
    let orders = table_id(&doc, "Orders");
    let create = |table, name: &str| CreateMeasure {
        table,
        measure: Measure::new(name, "1"),
        index: None,
    };
    let mut history = History::new();
    // Measure names are unique across the model, case-insensitively.
    assert!(matches!(
        history.execute(&mut doc, create(orders, "store count")),
        Err(CommandError::NameConflict { .. })
    ));
    // ... and differ from the table's columns.
    assert!(matches!(
        history.execute(&mut doc, create(stores, "Region")),
        Err(CommandError::NameConflict { .. })
    ));
    assert_eq!(
        history.execute(&mut doc, create(stores, " ")).unwrap_err(),
        CommandError::InvalidName(" ".into())
    );
    let measure = measure_id(&doc, "Stores", "Store Count");
    assert!(matches!(
        history.execute(&mut doc, create(measure, "X")),
        Err(CommandError::WrongKind { .. })
    ));
    assert_eq!(doc, original);
    assert_eq!(history.revision(), 0);
}

#[test]
fn delete_measure_round_trips_in_place() {
    let mut doc = sample_document();
    let count = measure_id(&doc, "Stores", "Store Count");
    assert_round_trip(&mut doc, DeleteMeasure { measure: count }, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert!(model.measure("Stores", "Store Count").is_none());
        assert_eq!(model.tables[0].measures[0].name, "Active Stores");
        assert!(doc.locate(count).is_none());
        assert_eq!(events, [ChangeEvent::ObjectRemoved { id: count }]);
    });
    assert_eq!(doc.model.as_ref().unwrap().tables[0].measures[0].id, count);
    let stores = table_id(&doc, "Stores");
    assert!(matches!(
        DeleteMeasure { measure: stores }.apply(&mut doc),
        Err(CommandError::WrongKind { .. })
    ));
}

#[test]
fn delete_column_round_trips_in_place() {
    let mut doc = sample_document();
    let status = column_id(&doc, "Stores", "Sales Status");
    assert_round_trip(&mut doc, DeleteColumn { column: status }, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert!(model.column("Stores", "Sales Status").is_none());
        assert!(doc.locate(status).is_none());
        assert_eq!(events, [ChangeEvent::ObjectRemoved { id: status }]);
    });
    assert_eq!(doc.model.as_ref().unwrap().tables[0].columns[4].id, status);
}

#[test]
fn delete_column_refuses_query_columns_and_structural_uses() {
    let mut doc = sample_document();
    let reject = |doc: &mut Document, table: &str, column: &str| {
        let id = column_id(doc, table, column);
        let before = doc.clone();
        let Err(CommandError::Rejected(message)) = DeleteColumn { column: id }.apply(doc) else {
            panic!("deleting {table}[{column}] should be rejected");
        };
        assert_eq!(*doc, before);
        message
    };
    assert!(reject(&mut doc, "Stores", "Store Name").contains("is a data column"));
    assert!(
        reject(&mut doc, "Unknown Store Orders", "Order ID")
            .contains("calculated table's expression")
    );
    let measure = measure_id(&doc, "Stores", "Store Count");
    assert!(matches!(
        DeleteColumn { column: measure }.apply(&mut doc),
        Err(CommandError::WrongKind { .. })
    ));

    // Calculated columns used by the model structure stay until those uses go.
    for column in &mut doc.model.as_mut().unwrap().tables[0].columns {
        column.kind = ColumnKind::Calculated {
            expression: "1".into(),
        };
    }
    assert_eq!(
        reject(&mut doc, "Stores", "Store ID"),
        "Stores[Store ID] is used by relationship Orders[Store ID] → Stores[Store ID]; delete or change those first"
    );
    assert!(
        reject(&mut doc, "Stores", "Store Name").contains("level 'Store' of hierarchy 'Geography'")
    );
    assert!(reject(&mut doc, "Stores", "Region Rank").contains("the sort order of Stores[Region]"));
}

#[test]
fn create_column_round_trips_and_checks() {
    let mut doc = sample_document();
    let original = doc.clone();
    let stores = table_id(&doc, "Stores");
    let column = |name: &str| Column::calculated(name, DataType::String, "Stores[Region] & \"!\"");
    let id = CreateColumn::created_id(&doc);
    let mut history = History::new();
    let events = history
        .execute(
            &mut doc,
            CreateColumn {
                table: stores,
                column: column("Shout"),
                index: None,
            },
        )
        .unwrap();
    assert_eq!(events, [ChangeEvent::ObjectAdded { id }]);
    assert_eq!(history.undo_label(), Some("Create column 'Shout'"));
    let model = doc.model.as_ref().unwrap();
    let created = model.tables[0].columns.last().unwrap();
    assert_eq!(
        (created.id, created.name.as_str(), created.expression()),
        (id, "Shout", Some("Stores[Region] & \"!\""))
    );
    let after = doc.clone();
    history.undo(&mut doc).unwrap();
    assert_same_content(&doc, &original);
    history.redo(&mut doc).unwrap();
    assert_eq!(doc, after, "redo restores the column with the same ID");

    let before = doc.clone();
    let measure = measure_id(&doc, "Stores", "Store Count");
    let mut refuse = |command: CreateColumn| {
        let error = command.apply(&mut doc).unwrap_err();
        assert_eq!(doc, before);
        error
    };
    assert!(matches!(
        refuse(CreateColumn {
            table: stores,
            column: column("region"),
            index: None
        }),
        CommandError::NameConflict { .. }
    ));
    assert!(matches!(
        refuse(CreateColumn {
            table: stores,
            column: column("Store Count"),
            index: None
        }),
        CommandError::NameConflict { .. }
    ));
    assert_eq!(
        refuse(CreateColumn {
            table: stores,
            column: column(" "),
            index: None
        }),
        CommandError::InvalidName(" ".into())
    );
    let CommandError::Rejected(message) = refuse(CreateColumn {
        table: stores,
        column: Column::data("Raw", DataType::String),
        index: None,
    }) else {
        panic!("a data column should be rejected");
    };
    assert!(message.contains("only calculated columns"), "{message}");
    assert!(matches!(
        refuse(CreateColumn {
            table: measure,
            column: column("X"),
            index: None
        }),
        CommandError::WrongKind { .. }
    ));
}
#[test]
fn delete_table_removes_its_relationships_and_role_filters() {
    let mut doc = sample_document();
    let stores = table_id(&doc, "Stores");
    let model = doc.model.as_ref().unwrap();
    let relationship = model.relationships[0].id;
    let permission = model.roles[0].table_permissions[0].id;
    let columns: Vec<ObjectId> = model.tables[0].columns.iter().map(|c| c.id).collect();
    assert_round_trip(&mut doc, DeleteTable { table: stores }, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert!(model.table("Stores").is_none());
        assert!(model.relationships.is_empty());
        assert!(model.roles[0].table_permissions.is_empty());
        assert_eq!(model.roles[0].name, "Region West");
        for id in [stores, relationship, permission].iter().chain(&columns) {
            assert!(doc.locate(*id).is_none());
        }
        assert_eq!(
            events,
            [
                ChangeEvent::ObjectRemoved { id: stores },
                ChangeEvent::ObjectRemoved { id: relationship },
                ChangeEvent::ObjectRemoved { id: permission },
            ]
        );
    });
    assert_eq!(doc.model.as_ref().unwrap().tables[0].id, stores);

    // A table without role filters takes only its relationships along.
    let orders = table_id(&doc, "Orders");
    assert_round_trip(&mut doc, DeleteTable { table: orders }, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert!(model.relationships.is_empty());
        assert_eq!(model.roles[0].table_permissions.len(), 1);
        assert_eq!(events.len(), 2);
    });
    assert!(matches!(
        DeleteTable {
            table: relationship
        }
        .apply(&mut doc),
        Err(CommandError::WrongKind { .. })
    ));
}
#[test]
fn create_relationship_round_trips_and_never_reuses_ids() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut relationship = Relationship::new(
        "r-new",
        ColumnRef::new("Unknown Store Orders", "Order ID"),
        ColumnRef::new("Orders", "Order ID"),
    );
    relationship.id = ObjectId(424_242); // ignored
    let id = CreateRelationship::created_id(&doc);
    let mut history = History::new();
    let events = history
        .execute(
            &mut doc,
            CreateRelationship {
                relationship,
                index: Some(0),
            },
        )
        .unwrap();
    assert_eq!(events, [ChangeEvent::ObjectAdded { id }]);
    assert_eq!(
        history.undo_label(),
        Some("Create relationship 'Unknown Store Orders'[Order ID] → Orders[Order ID]")
    );
    let relationships = &doc.model.as_ref().unwrap().relationships;
    assert_eq!(
        (
            relationships.len(),
            relationships[0].id,
            relationships[0].name.as_str()
        ),
        (2, id, "r-new")
    );
    assert!(matches!(
        doc.locate(id),
        Some(ObjectPath::Relationship { relationship: 0 })
    ));
    let after = doc.clone();

    assert_eq!(
        history.undo(&mut doc).unwrap(),
        [ChangeEvent::ObjectRemoved { id }]
    );
    assert_same_content(&doc, &original);
    history.redo(&mut doc).unwrap();
    assert_eq!(
        doc, after,
        "redo restores the relationship with the same ID"
    );
    history.undo(&mut doc).unwrap();
    assert!(CreateRelationship::created_id(&doc) > id);
}

#[test]
fn delete_relationship_round_trips_in_place() {
    let mut doc = sample_document();
    let id = doc.model.as_ref().unwrap().relationships[0].id;
    assert_round_trip(
        &mut doc,
        DeleteRelationship { relationship: id },
        |doc, events| {
            assert!(doc.model.as_ref().unwrap().relationships.is_empty());
            assert!(doc.locate(id).is_none());
            assert_eq!(events, [ChangeEvent::ObjectRemoved { id }]);
        },
    );
    let orders = table_id(&doc, "Orders");
    assert!(matches!(
        DeleteRelationship {
            relationship: orders
        }
        .apply(&mut doc),
        Err(CommandError::WrongKind { .. })
    ));
}

#[test]
fn invalid_relationships_fail_without_changing_the_document() {
    let mut doc = sample_document();
    let original = doc.clone();
    let create = |name: &str, from: (&str, &str), to: (&str, &str), active: bool| {
        let mut r = Relationship::new(
            name,
            ColumnRef::new(from.0, from.1),
            ColumnRef::new(to.0, to.1),
        );
        r.is_active = active;
        CreateRelationship {
            relationship: r,
            index: None,
        }
    };
    let rejected = |result: Result<Vec<ChangeEvent>, CommandError>, needle: &str| match result {
        Err(CommandError::Rejected(message)) => {
            assert!(message.contains(needle), "{message:?} lacks {needle:?}")
        }
        other => panic!("expected a rejection with {needle:?}, got {other:?}"),
    };
    let mut history = History::new();
    assert!(matches!(
        history.execute(
            &mut doc,
            create(
                "D0C1E2F3-A4B5-4C6D-8E7F-000000000001",
                ("Orders", "Order ID"),
                ("Stores", "Store Name"),
                false,
            ),
        ),
        Err(CommandError::NameConflict { .. })
    ));
    assert_eq!(
        history
            .execute(
                &mut doc,
                create(" ", ("Orders", "Order ID"), ("Stores", "Store Name"), false)
            )
            .unwrap_err(),
        CommandError::InvalidName(" ".into())
    );
    rejected(
        history.execute(
            &mut doc,
            create("r", ("Orders", "Nope"), ("Stores", "Store ID"), false),
        ),
        "no column Orders[Nope]",
    );
    rejected(
        history.execute(
            &mut doc,
            create("r", ("Orders", "Store ID"), ("orders", "Order ID"), false),
        ),
        "two different tables",
    );
    // The same two columns, in either direction.
    rejected(
        history.execute(
            &mut doc,
            create("r", ("Stores", "store id"), ("Orders", "Store ID"), false),
        ),
        "already joins these columns",
    );
    rejected(
        history.execute(
            &mut doc,
            create("r", ("Orders", "Order ID"), ("Stores", "Store Name"), true),
        ),
        "already have an active relationship",
    );
    assert_eq!(doc, original);
    assert_eq!(history.revision(), 0);

    // Inactive is fine.
    history
        .execute(
            &mut doc,
            create("r", ("Orders", "Order ID"), ("Stores", "Store Name"), false),
        )
        .unwrap();
    assert_eq!(doc.model.as_ref().unwrap().relationships.len(), 2);
}

#[test]
fn set_relationship_properties_round_trips_and_checks() {
    use crate::semantic::{Cardinality, CrossFilter, SecurityFilteringBehavior};
    let mut doc = sample_document();
    let stores = doc.model.as_ref().unwrap().relationships[0].id;
    let both_ways = SetRelationshipProperties {
        relationship: stores,
        to_cardinality: Some(Cardinality::Many),
        cross_filtering_behavior: Some(CrossFilter::BothDirections),
        security_filtering_behavior: Some(SecurityFilteringBehavior::BothDirections),
        is_active: Some(false),
        ..SetRelationshipProperties::default()
    };
    assert_round_trip(&mut doc, both_ways, |doc, events| {
        let r = &doc.model.as_ref().unwrap().relationships[0];
        assert_eq!(
            (
                &r.from_cardinality,
                &r.to_cardinality,
                &r.cross_filtering_behavior,
                &r.security_filtering_behavior,
                r.is_active
            ),
            (
                &Cardinality::Many,
                &Cardinality::Many,
                &CrossFilter::BothDirections,
                &SecurityFilteringBehavior::BothDirections,
                false
            )
        );
        assert_eq!(events, [ChangeEvent::ObjectChanged { id: stores }]);
    });
    let mut history = History::new();
    let same = SetRelationshipProperties {
        relationship: stores,
        is_active: Some(true),
        ..SetRelationshipProperties::default()
    };
    assert!(history.execute(&mut doc, same).unwrap().is_empty());

    let rejected = |result: Result<Vec<ChangeEvent>, CommandError>, needle: &str| match result {
        Err(CommandError::Rejected(message)) => {
            assert!(message.contains(needle), "{message:?} lacks {needle:?}")
        }
        other => panic!("expected a rejection with {needle:?}, got {other:?}"),
    };
    let original = doc.clone();
    let security_only = SetRelationshipProperties {
        relationship: stores,
        cross_filtering_behavior: Some(CrossFilter::OneDirection),
        security_filtering_behavior: Some(SecurityFilteringBehavior::BothDirections),
        ..SetRelationshipProperties::default()
    };
    rejected(
        history.execute(&mut doc, security_only),
        "set crossFilteringBehavior to bothDirections too",
    );
    let table = table_id(&doc, "Orders");
    assert!(matches!(
        history.execute(
            &mut doc,
            SetRelationshipProperties {
                relationship: table,
                ..SetRelationshipProperties::default()
            }
        ),
        Err(CommandError::WrongKind { .. })
    ));

    // A second relationship between Orders and Stores can only become
    // active once the first is not; in one group that is one undo step.
    let mut names = Relationship::new(
        "names",
        ColumnRef::new("Orders", "Order ID"),
        ColumnRef::new("Stores", "Store Name"),
    );
    names.is_active = false;
    history
        .execute(
            &mut doc,
            CreateRelationship {
                relationship: names,
                index: None,
            },
        )
        .unwrap();
    let names = doc.model.as_ref().unwrap().relationships[1].id;
    let activate = SetRelationshipProperties {
        relationship: names,
        is_active: Some(true),
        ..SetRelationshipProperties::default()
    };
    let before_swap = doc.clone();
    rejected(
        history.execute(&mut doc, activate.clone()),
        "make that one inactive first",
    );
    assert_eq!(doc, before_swap);
    let swap = CommandGroup::new("Swap the active relationship")
        .with(SetRelationshipProperties {
            relationship: stores,
            is_active: Some(false),
            ..SetRelationshipProperties::default()
        })
        .with(activate);
    history.execute(&mut doc, swap).unwrap();
    let active: Vec<bool> = doc
        .model
        .as_ref()
        .unwrap()
        .relationships
        .iter()
        .map(|r| r.is_active)
        .collect();
    assert_eq!(active, [false, true]);
    history.undo(&mut doc).unwrap();
    assert_eq!(doc, before_swap);
    history.undo(&mut doc).unwrap();
    assert_same_content(&doc, &original);

    // Undo restores what was there without the checks: here two active
    // relationships between the same tables, as a file may have them.
    let mut doc = sample_document();
    let mut second = Relationship::new(
        "second",
        ColumnRef::new("Orders", "Order ID"),
        ColumnRef::new("Stores", "Store Name"),
    );
    second.id = doc.ids.next_id();
    let second_id = second.id;
    doc.model.as_mut().unwrap().relationships.push(second);
    assert_round_trip(
        &mut doc,
        SetRelationshipProperties {
            relationship: second_id,
            is_active: Some(false),
            ..SetRelationshipProperties::default()
        },
        |doc, _| assert!(!doc.model.as_ref().unwrap().relationships[1].is_active),
    );
}

#[test]
fn set_measure_properties_patches_only_given_fields() {
    let mut doc = sample_document();
    let count = measure_id(&doc, "Stores", "Store Count");
    assert_round_trip(
        &mut doc,
        SetMeasureProperties {
            measure: count,
            expression: Some("COUNTROWS(Stores)".into()),
            format_string: Some(None),
            display_folder: Some(Some("KPIs".into())),
            is_hidden: Some(true),
            ..SetMeasureProperties::default()
        },
        |doc, events| {
            let m = doc.model.as_ref().unwrap().measure("Stores", "Store Count");
            let m = m.unwrap();
            assert_eq!(m.expression, "COUNTROWS(Stores)");
            assert_eq!(m.format_string, None);
            assert_eq!(m.display_folder.as_deref(), Some("KPIs"));
            assert!(m.is_hidden);
            assert_eq!(m.description.as_deref(), Some("Distinct count of stores."));
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: count }]);
        },
    );

    let mut history = History::new();
    let unchanged = SetMeasureProperties {
        measure: count,
        format_string: Some(Some("0".into())),
        ..SetMeasureProperties::default()
    };
    assert!(
        !history
            .execute_with_status(&mut doc, unchanged)
            .unwrap()
            .changed
    );

    let active = measure_id(&doc, "Stores", "Active Stores");
    let measures = &mut doc.model.as_mut().unwrap().tables[0].measures;
    measures[1].extra.insert(
        "formatStringDefinition".into(),
        crate::props::PropValue::Expression("\"0.0\"".into()),
    );
    let original = doc.clone();
    let err = SetMeasureProperties {
        measure: active,
        format_string: Some(Some("0".into())),
        ..SetMeasureProperties::default()
    }
    .apply(&mut doc)
    .unwrap_err();
    assert!(matches!(err, CommandError::Rejected(m) if m.contains("dynamic format string")));
    assert_eq!(doc, original);
}

#[test]
fn set_column_properties_patches_calculated_columns_only() {
    let mut doc = sample_document();
    let status = column_id(&doc, "Stores", "Sales Status");
    assert_round_trip(
        &mut doc,
        SetColumnProperties {
            column: status,
            expression: Some("IF(ISEMPTY(RELATEDTABLE(Orders)), 0, 1)".into()),
            data_type: Some(DataType::Int64),
            summarize_by: Some(Some(SummarizeBy::None)),
            display_folder: Some(Some("Flags".into())),
            is_hidden: Some(true),
            ..SetColumnProperties::default()
        },
        |doc, events| {
            let c = doc.model.as_ref().unwrap().column("Stores", "Sales Status");
            let c = c.unwrap();
            assert_eq!(
                c.expression(),
                Some("IF(ISEMPTY(RELATEDTABLE(Orders)), 0, 1)")
            );
            assert_eq!(c.data_type, DataType::Int64);
            assert_eq!(c.summarize_by, Some(SummarizeBy::None));
            assert_eq!(c.display_folder.as_deref(), Some("Flags"));
            assert!(c.is_hidden);
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: status }]);
        },
    );

    let mut history = History::new();
    let unchanged = SetColumnProperties {
        column: status,
        data_type: Some(DataType::String),
        ..SetColumnProperties::default()
    };
    assert!(
        !history
            .execute_with_status(&mut doc, unchanged)
            .unwrap()
            .changed
    );

    let name = column_id(&doc, "Stores", "Store Name");
    assert_round_trip(
        &mut doc,
        SetColumnProperties {
            column: name,
            description: Some(Some("The store's name.".into())),
            ..SetColumnProperties::default()
        },
        |doc, _| {
            let c = doc.model.as_ref().unwrap().column("Stores", "Store Name");
            assert_eq!(c.unwrap().description.as_deref(), Some("The store's name."));
        },
    );
    let original = doc.clone();
    for command in [
        SetColumnProperties {
            column: name,
            expression: Some("1".into()),
            ..SetColumnProperties::default()
        },
        SetColumnProperties {
            column: name,
            data_type: Some(DataType::Int64),
            ..SetColumnProperties::default()
        },
    ] {
        let err = command.apply(&mut doc).unwrap_err();
        assert!(matches!(err, CommandError::Rejected(m) if m.contains("not a calculated column")));
        assert_eq!(doc, original);
    }
    let measure = measure_id(&doc, "Stores", "Store Count");
    let err = SetColumnProperties {
        column: measure,
        is_hidden: Some(true),
        ..SetColumnProperties::default()
    }
    .apply(&mut doc)
    .unwrap_err();
    assert!(matches!(err, CommandError::WrongKind { .. }), "{err:?}");
}

#[test]
fn replace_objects_validates_targets_first() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut table = doc.model.as_ref().unwrap().tables[0].clone();
    table.is_hidden = true;
    let mut missing = table.clone();
    missing.id = ObjectId(999_999);
    let command = ReplaceObjects {
        label: "Replace".into(),
        items: vec![Snapshot::Table(table), Snapshot::Table(missing)],
    };
    assert_eq!(
        command.apply(&mut doc).unwrap_err(),
        CommandError::NotFound(ObjectId(999_999))
    );
    assert_eq!(doc, original);
}

#[test]
fn change_events_serialize_for_ipc() {
    let event = ChangeEvent::ObjectRenamed {
        id: ObjectId(7),
        old: "a".into(),
        new: "b".into(),
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"objectRenamed": {"id": 7, "old": "a", "new": "b"}})
    );
    let back: ChangeEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, event);
}

#[test]
fn hierarchies_are_created_and_deleted_with_exact_inverses() {
    use crate::semantic::{Hierarchy, Level};
    let mut doc = sample_document();
    let original = doc.clone();
    let stores = table_id(&doc, "Stores");
    let level = |name: &str, column: &str| Level {
        name: name.into(),
        column: column.into(),
        ..Level::default()
    };
    let hierarchy = |name: &str, levels: Vec<Level>| Hierarchy {
        name: name.into(),
        lineage_tag: Some("tag".into()),
        levels,
        ..Hierarchy::default()
    };
    let id = CreateHierarchy::created_id(&doc);
    let mut history = History::new();
    let events = history
        .execute(
            &mut doc,
            CreateHierarchy {
                table: stores,
                hierarchy: hierarchy(
                    "Places",
                    vec![level("Region", "Region"), level("Store", "Store Name")],
                ),
                index: None,
            },
        )
        .unwrap();
    assert_eq!(events, [ChangeEvent::ObjectAdded { id }]);
    assert_eq!(history.undo_label(), Some("Create hierarchy 'Places'"));
    let created = doc.model.as_ref().unwrap().tables[0]
        .hierarchies
        .last()
        .unwrap()
        .clone();
    assert_eq!((created.id, created.name.as_str()), (id, "Places"));
    let levels: Vec<_> = created
        .levels
        .iter()
        .map(|l| (l.id, l.column.as_str()))
        .collect();
    assert_eq!(
        levels,
        [
            (ObjectId(id.0 + 1), "Region"),
            (ObjectId(id.0 + 2), "Store Name")
        ]
    );
    assert!(doc.ids.peek() > ObjectId(id.0 + 2));
    let after = doc.clone();
    history.undo(&mut doc).unwrap();
    assert_same_content(&doc, &original);
    history.redo(&mut doc).unwrap();
    assert_eq!(doc, after, "redo restores the hierarchy with the same IDs");

    let before = doc.clone();
    let mut refuse = |levels: Vec<Level>, name: &str| {
        let error = CreateHierarchy {
            table: stores,
            hierarchy: hierarchy(name, levels),
            index: None,
        }
        .apply(&mut doc)
        .unwrap_err();
        assert_eq!(doc, before);
        error.to_string()
    };
    let one = || vec![level("Region", "Region")];
    for taken in ["region", "geography", "Store Count"] {
        assert!(refuse(one(), taken).contains("already"), "{taken}");
    }
    assert!(refuse(Vec::new(), "H").contains("has no levels"));
    assert!(refuse(vec![level("X", "Nope")], "H").contains("has no column 'Nope'"));
    assert!(
        refuse(vec![level("A", "Region"), level("B", "region")], "H")
            .contains("use the same column")
    );
    assert!(refuse(vec![level("A", "Region"), level("a", "Store Name")], "H").contains("already"));

    // Delete and undo put Geography back exactly.
    let geography = doc.model.as_ref().unwrap().tables[0].hierarchies[0].id;
    let before = doc.clone();
    let events = history
        .execute(
            &mut doc,
            DeleteHierarchy {
                hierarchy: geography,
            },
        )
        .unwrap();
    assert_eq!(events, [ChangeEvent::ObjectRemoved { id: geography }]);
    assert!(doc.locate(geography).is_none());
    history.undo(&mut doc).unwrap();
    assert_eq!(doc, before);
    assert!(matches!(
        DeleteHierarchy { hierarchy: stores }.apply(&mut doc),
        Err(CommandError::WrongKind { .. })
    ));
}

#[test]
fn calculated_tables_are_created_with_fresh_ids_and_deleted_by_undo() {
    use crate::semantic::{Partition, PartitionMode, PartitionSource, Table};
    let mut doc = sample_document();
    let original = doc.clone();
    let from_expression = |name: &str| Column {
        name: name.into(),
        data_type: DataType::String,
        is_name_inferred: true,
        kind: ColumnKind::CalculatedTableColumn {
            source_column: format!("[{name}]"),
        },
        ..Column::default()
    };
    let table = |name: &str, columns: Vec<Column>, source: PartitionSource| Table {
        name: name.into(),
        lineage_tag: Some("tag".into()),
        columns,
        partitions: vec![Partition {
            name: name.into(),
            mode: PartitionMode::Import,
            source,
            ..Partition::default()
        }],
        ..Table::default()
    };
    let dax = || PartitionSource::Calculated {
        expression: "VALUES(Stores[Region])".into(),
    };
    let id = CreateTable::created_id(&doc);
    let mut history = History::new();
    let events = history
        .execute(
            &mut doc,
            CreateTable {
                table: table(
                    "Regions",
                    vec![
                        from_expression("Region"),
                        Column::calculated("Upper", DataType::String, "UPPER(Regions[Region])"),
                    ],
                    dax(),
                ),
                index: None,
            },
        )
        .unwrap();
    assert_eq!(events, [ChangeEvent::ObjectAdded { id }]);
    assert_eq!(history.undo_label(), Some("Create table 'Regions'"));
    let created = doc.model.as_ref().unwrap().tables.last().unwrap().clone();
    assert_eq!((created.id, created.name.as_str()), (id, "Regions"));
    let ids: Vec<_> = created
        .columns
        .iter()
        .map(|c| c.id)
        .chain(created.partitions.iter().map(|p| p.id))
        .collect();
    assert_eq!(
        ids,
        [ObjectId(id.0 + 1), ObjectId(id.0 + 2), ObjectId(id.0 + 3)]
    );
    assert!(doc.ids.peek() > ObjectId(id.0 + 3));
    let after = doc.clone();
    history.undo(&mut doc).unwrap();
    assert_same_content(&doc, &original);
    history.redo(&mut doc).unwrap();
    assert_eq!(doc, after, "redo restores the table with the same IDs");

    let before = doc.clone();
    let mut refuse = |t: Table| {
        let error = CreateTable {
            table: t,
            index: Some(0),
        }
        .apply(&mut doc)
        .unwrap_err();
        assert_eq!(doc, before);
        error.to_string()
    };
    let one = || vec![from_expression("Region")];
    assert!(refuse(table("stores", one(), dax())).contains("already"));
    assert!(refuse(table("regions", one(), dax())).contains("already"));
    let m = PartitionSource::M {
        expression: "let Source = 1 in Source".into(),
    };
    assert!(refuse(table("T", one(), m)).contains("only calculated tables"));
    assert!(refuse(table("T", Vec::new(), dax())).contains("has no columns"));
    let twice = vec![from_expression("Region"), from_expression("region")];
    assert!(refuse(table("T", twice, dax())).contains("already"));
    let data = vec![Column::data("Region", DataType::String)];
    assert!(refuse(table("T", data, dax())).contains("come from its expression"));
    let mut with_measure = table("T", one(), dax());
    with_measure.measures.push(Measure::new("M", "1"));
    assert!(refuse(with_measure).contains("after the table"));
}

#[test]
fn table_definitions_change_columns_with_exact_inverses() {
    use crate::semantic::{PartitionSource, Table};
    let mut doc = sample_document();
    let id = table_id(&doc, "Unknown Store Orders");
    let order_id = column_id(&doc, "Unknown Store Orders", "Order ID");
    let partition = doc
        .model
        .as_ref()
        .unwrap()
        .table("Unknown Store Orders")
        .unwrap()
        .partitions[0]
        .id;
    let current = |doc: &Document| -> Table {
        doc.model
            .as_ref()
            .unwrap()
            .table("Unknown Store Orders")
            .unwrap()
            .clone()
    };
    let from_expression = |name: &str, source: &str| Column {
        name: name.into(),
        data_type: DataType::String,
        is_name_inferred: true,
        kind: ColumnKind::CalculatedTableColumn {
            source_column: source.into(),
        },
        ..Column::default()
    };
    let set_expression = |t: &mut Table, expression: &str| {
        t.partitions[0].source = PartitionSource::Calculated {
            expression: expression.into(),
        };
    };

    // Like `assert_round_trip`, but undo keeps the ID generator.
    let round_trip = |doc: &mut Document,
                      command: SetTableDefinition,
                      check: &dyn Fn(&Document, &[ChangeEvent])| {
        let original = doc.clone();
        let mut history = History::new();
        let events = history.execute(doc, command).unwrap();
        check(doc, &events);
        let after = doc.clone();
        history.undo(doc).unwrap();
        assert_same_content(doc, &original);
        history.redo(doc).unwrap();
        assert_eq!(*doc, after, "redo keeps the new columns' IDs");
        history.undo(doc).unwrap();
        assert_same_content(doc, &original);
    };

    // A new expression: the kept column changes type and source, a new one
    // gets the next ID.
    let mut changed = current(&doc);
    set_expression(
        &mut changed,
        "SUMMARIZE(Orders, Orders[Order ID], Orders[Store ID])",
    );
    changed.columns[0].data_type = DataType::Int64;
    changed.columns[0].kind = ColumnKind::CalculatedTableColumn {
        source_column: "Orders[Order ID]".into(),
    };
    changed
        .columns
        .push(from_expression("Store ID", "Orders[Store ID]"));
    changed.description = Some("Orders and their stores".into());
    let added = doc.ids.peek();
    round_trip(
        &mut doc,
        SetTableDefinition {
            table: changed.clone(),
        },
        &|doc, events| {
            assert_eq!(
                events,
                [
                    ChangeEvent::ObjectChanged { id },
                    ChangeEvent::ObjectChanged { id: order_id },
                    ChangeEvent::ObjectAdded { id: added },
                    ChangeEvent::ObjectChanged { id: partition },
                ]
            );
            let after = current(doc);
            assert_eq!(after.columns[1].id, added);
            assert_eq!(
                after.calculated_expression(),
                changed.calculated_expression()
            );
            assert_eq!(
                after.description.as_deref(),
                Some("Orders and their stores")
            );
        },
    );

    // A column the expression no longer returns is removed (undo puts it
    // back, ID included).
    let mut replaced = current(&doc);
    set_expression(&mut replaced, "VALUES(Orders[Store ID])");
    replaced.columns = vec![from_expression("Store ID", "Orders[Store ID]")];
    let store_id = doc.ids.peek();
    round_trip(
        &mut doc,
        SetTableDefinition { table: replaced },
        &|doc, events| {
            assert_eq!(
                events,
                [
                    ChangeEvent::ObjectChanged { id },
                    ChangeEvent::ObjectAdded { id: store_id },
                    ChangeEvent::ObjectRemoved { id: order_id },
                    ChangeEvent::ObjectChanged { id: partition },
                ]
            );
            assert!(doc.locate(order_id).is_none());
        },
    );

    // Any table's own properties.
    let mut stores = doc.model.as_ref().unwrap().table("Stores").unwrap().clone();
    stores.is_hidden = true;
    let stores_id = stores.id;
    assert_round_trip(
        &mut doc,
        SetTableDefinition { table: stores },
        |_, events| {
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: stores_id }]);
        },
    );

    // Refusals leave the document unchanged.
    doc.model
        .as_mut()
        .unwrap()
        .relationships
        .push(Relationship::new(
            "d0c1e2f3-a4b5-4c6d-8e7f-000000000099",
            ColumnRef::new("Unknown Store Orders", "Order ID"),
            ColumnRef::new("Orders", "Order ID"),
        ));
    let before = doc.clone();
    let mut refuse = |t: Table| {
        let error = SetTableDefinition { table: t }.apply(&mut doc).unwrap_err();
        assert_eq!(doc, before);
        error.to_string()
    };
    let base = current(&before);
    let edited = |f: &dyn Fn(&mut Table)| {
        let mut t = base.clone();
        f(&mut t);
        t
    };
    assert!(refuse(edited(&|t| t.name = "Other".into())).contains("rename"));
    assert!(refuse(edited(&|t| t.measures.push(Measure::new("M", "1")))).contains("own commands"));
    assert!(
        refuse(edited(&|t| t.partitions[0].source = PartitionSource::M {
            expression: "1".into()
        }))
        .contains("partition")
    );
    assert!(refuse(edited(&|t| t.columns[0].name = "Renamed".into())).contains("rename"));
    assert!(
        refuse(edited(&|t| t
            .columns
            .push(Column::data("Raw", DataType::String))))
        .contains("must come from the expression")
    );
    assert!(
        refuse(edited(&|t| t
            .columns
            .push(from_expression("order id", "[order id]"))))
        .contains("already")
    );
    assert!(refuse(edited(&|t| t.columns.clear())).contains("at least one column"));
    let mut orders = before
        .model
        .as_ref()
        .unwrap()
        .table("Orders")
        .unwrap()
        .clone();
    orders.columns.pop();
    assert!(refuse(orders).contains("not a calculated table"));
    let foreign = column_id(&before, "Stores", "Region");
    assert!(
        refuse(edited(&|t| {
            let mut c = from_expression("Region", "[Region]");
            c.id = foreign;
            t.columns.push(c);
        }))
        .contains("not one of its columns")
    );
    assert!(
        refuse(edited(&|t| {
            set_expression(t, "VALUES(Orders[Store ID])");
            t.columns = vec![from_expression("Store ID", "Orders[Store ID]")];
        }))
        .contains("used by relationship")
    );
}

#[test]
fn renames_follow_report_level_measures_whose_names_are_taken() {
    use crate::report::{MeasureReference, ReportMeasure};
    let reference = |schema: &str, table: &str, name: &str| MeasureReference {
        schema: schema.into(),
        table: table.into(),
        name: name.into(),
    };
    let mut doc = sample_document();
    doc.reports[0].measures = vec![
        ReportMeasure {
            table: "Stores".into(),
            name: "Half Count".into(),
            expression: "[Store Count] / 2".into(),
            references: vec![reference("", "Stores", "Store Count")],
            ..ReportMeasure::default()
        },
        ReportMeasure {
            table: "Orders".into(),
            name: "Full Count".into(),
            expression: "[Half Count] * 2".into(),
            references: vec![reference("extension", "Stores", "Half Count")],
            ..ReportMeasure::default()
        },
    ];
    let stores = table_id(&doc, "Stores");
    let command = RenameTable {
        table: stores,
        new_name: "Shops".into(),
    };
    assert_round_trip(&mut doc, command, |doc, events| {
        let m = &doc.reports[0].measures;
        assert_eq!(
            (m[0].table.as_str(), m[1].table.as_str()),
            ("Shops", "Orders")
        );
        assert_eq!(m[0].references, [reference("", "Shops", "Store Count")]);
        assert_eq!(
            m[1].references,
            [reference("extension", "Shops", "Half Count")]
        );
        // The DAX is rewritten by pbi-session's rename.
        assert_eq!(m[0].expression, "[Store Count] / 2");
        let report = doc.reports[0].id;
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: report }));
    });
    let count = measure_id(&doc, "Stores", "Store Count");
    let command = RenameMeasure {
        measure: count,
        new_name: "Stores Total".into(),
    };
    assert_round_trip(&mut doc, command, |doc, _| {
        let m = &doc.reports[0].measures;
        assert_eq!(m[0].references, [reference("", "Stores", "Stores Total")]);
        assert_eq!(
            m[1].references,
            [reference("extension", "Stores", "Half Count")]
        );
    });

    // A report-level measure's name is taken for measures (anywhere) and
    // for columns of the table it is shown in.
    let original = doc.clone();
    let mut history = History::new();
    let (orders, region) = (
        table_id(&doc, "Orders"),
        column_id(&doc, "Stores", "Region"),
    );
    let order_id = column_id(&doc, "Orders", "Order ID");
    let conflict = |result: Result<Vec<ChangeEvent>, CommandError>| {
        assert!(
            matches!(&result, Err(CommandError::NameConflict { .. })),
            "{result:?}"
        );
    };
    conflict(history.execute(
        &mut doc,
        RenameMeasure {
            measure: count,
            new_name: "half count".into(),
        },
    ));
    conflict(history.execute(
        &mut doc,
        CreateMeasure {
            table: orders,
            measure: Measure::new("Full Count", "1"),
            index: None,
        },
    ));
    conflict(history.execute(
        &mut doc,
        RenameColumn {
            column: region,
            new_name: "Half Count".into(),
        },
    ));
    assert_eq!(doc, original);
    // Another table's column may have that name.
    history
        .execute(
            &mut doc,
            RenameColumn {
                column: order_id,
                new_name: "Half Count".into(),
            },
        )
        .unwrap();
}
